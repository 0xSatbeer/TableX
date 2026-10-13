use crate::ddl::{TableEdit, alter_table, create_table};
use crate::model::{Column, ForeignKey, Index, TableDef};

/// The referential actions an FK's on delete and on update choose from.
pub const FK_ACTIONS: [&str; 5] = [
    "NO ACTION",
    "RESTRICT",
    "CASCADE",
    "SET NULL",
    "SET DEFAULT",
];

/// The table editor's state, apart from its inputs: the definition it opened on and the edits.
#[derive(Clone, Debug, PartialEq)]
pub struct TableForm {
    pub before: TableDef,
    pub name: String,
    /// Each column with its original name, `None` when added, so a rename stays a rename.
    pub columns: Vec<(Option<String>, Column)>,
    pub indexes: Vec<Index>,
    pub foreign_keys: Vec<ForeignKey>,
}

impl TableForm {
    pub fn modify(before: TableDef) -> Self {
        let columns = before
            .columns
            .iter()
            .map(|c| (Some(c.name.clone()), c.clone()))
            .collect();
        Self {
            name: before.name.clone(),
            indexes: before.indexes.clone(),
            foreign_keys: before.foreign_keys.clone(),
            before,
            columns,
        }
    }

    /// A new table: no name, one empty column. Its `before` is blank, which marks create mode.
    pub fn create() -> Self {
        let mut form = Self::modify(TableDef {
            name: String::new(),
            columns: Vec::new(),
            indexes: Vec::new(),
            foreign_keys: Vec::new(),
        });
        form.add_column();
        form
    }

    pub fn is_create(&self) -> bool {
        self.before.name.is_empty()
    }

    pub fn add_column(&mut self) {
        let column = Column {
            name: String::new(),
            data_type: String::new(),
            nullable: true,
            default: None,
            is_pk: false,
            enum_values: Vec::new(),
        };
        self.columns.push((None, column));
    }

    pub fn add_index(&mut self) {
        self.indexes.push(Index {
            name: String::new(),
            columns: Vec::new(),
            unique: false,
        });
    }

    /// A new FK refers to `schema`, the table's own, until edited.
    pub fn add_foreign_key(&mut self, schema: &str) {
        self.foreign_keys.push(ForeignKey {
            name: String::new(),
            columns: Vec::new(),
            ref_schema: schema.into(),
            ref_table: String::new(),
            ref_columns: Vec::new(),
            on_delete: FK_ACTIONS[0].into(),
            on_update: FK_ACTIONS[0].into(),
        });
    }

    /// The statements the edits need; none while a required field is blank.
    pub fn statements(&self, schema: &str) -> Vec<String> {
        let blank_column = self
            .columns
            .iter()
            .any(|(_, c)| c.name.is_empty() || c.data_type.is_empty());
        let blank_index = self
            .indexes
            .iter()
            .any(|i| i.name.is_empty() || i.columns.is_empty());
        let blank_fk = self.foreign_keys.iter().any(|f| {
            f.name.is_empty()
                || f.columns.is_empty()
                || f.ref_schema.is_empty()
                || f.ref_table.is_empty()
                || f.ref_columns.is_empty()
        });
        if self.name.is_empty() || blank_column || blank_index || blank_fk {
            return Vec::new();
        }
        if self.is_create() {
            return create_table(schema, &self.edit().def);
        }
        alter_table(schema, &self.before, &self.edit())
    }

    fn edit(&self) -> TableEdit {
        let (origins, columns) = self.columns.iter().cloned().unzip();
        TableEdit {
            def: TableDef {
                name: self.name.clone(),
                columns,
                indexes: self.indexes.clone(),
                foreign_keys: self.foreign_keys.clone(),
            },
            origins,
        }
    }
}

/// `"a, b"` as `["a", "b"]`; blanks dropped.
pub fn names(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(String::from)
        .collect()
}
