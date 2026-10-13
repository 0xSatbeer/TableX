use crate::ddl::{TableEdit, alter_table, create_table};
use crate::model::{Column, ForeignKey, Index, TableDef};

/// The types the Type field's menu offers, before the ones the table already uses.
pub const COMMON_TYPES: [&str; 22] = [
    "bigint",
    "integer",
    "smallint",
    "bigserial",
    "serial",
    "numeric(10, 2)",
    "double precision",
    "real",
    "boolean",
    "text",
    "varchar(255)",
    "char(1)",
    "uuid",
    "date",
    "time",
    "timestamp",
    "timestamp with time zone",
    "interval",
    "jsonb",
    "json",
    "bytea",
    "inet",
];

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
            primary_key: None,
            unique_keys: Vec::new(),
        });
        form.add_column();
        form
    }

    /// The Type menu: the common types, then the others this table uses (enums, domains).
    pub fn type_choices(&self) -> Vec<String> {
        let mut choices: Vec<String> = COMMON_TYPES.iter().map(|t| t.to_string()).collect();
        for (_, c) in &self.columns {
            if !c.data_type.is_empty() && !choices.contains(&c.data_type) {
                choices.push(c.data_type.clone());
            }
        }
        choices
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
            identity: false,
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

    /// Why Apply has nothing to run yet: a required field is blank.
    pub fn blocker(&self) -> Option<&'static str> {
        if self.name.is_empty() {
            return Some("Name the table.");
        }
        if self
            .columns
            .iter()
            .any(|(_, c)| c.name.is_empty() || c.data_type.is_empty())
        {
            return Some("Every column needs a name and a type.");
        }
        if self
            .indexes
            .iter()
            .any(|i| i.name.is_empty() || i.columns.is_empty())
        {
            return Some("Every index needs a name and columns.");
        }
        let blank_fk = self.foreign_keys.iter().any(|f| {
            f.name.is_empty()
                || f.columns.is_empty()
                || f.ref_schema.is_empty()
                || f.ref_table.is_empty()
                || f.ref_columns.is_empty()
        });
        blank_fk.then_some("Every foreign key needs a name, columns and what it references.")
    }

    /// The statements the edits need; none while a required field is blank.
    pub fn statements(&self, schema: &str) -> Vec<String> {
        if self.blocker().is_some() {
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
                primary_key: self.before.primary_key.clone(),
                unique_keys: self.before.unique_keys.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_choices_add_the_tables_own_types_once_after_the_common_ones() {
        let mut form = TableForm::create();
        form.columns[0].1.data_type = "enum_status".into();
        form.add_column();
        form.columns[1].1.data_type = "integer".into();
        form.add_column();
        form.columns[2].1.data_type = "enum_status".into();
        let choices = form.type_choices();
        assert_eq!(choices.len(), COMMON_TYPES.len() + 1);
        assert_eq!(choices.last().map(String::as_str), Some("enum_status"));
    }

    #[test]
    fn blocker_names_the_first_blank_required_field() {
        let mut form = TableForm::create();
        assert_eq!(form.blocker(), Some("Name the table."));
        form.name = "t".into();
        assert_eq!(
            form.blocker(),
            Some("Every column needs a name and a type.")
        );
        form.columns[0].1.name = "a".into();
        form.columns[0].1.data_type = "int".into();
        assert_eq!(form.blocker(), None);
        form.add_index();
        assert_eq!(
            form.blocker(),
            Some("Every index needs a name and columns.")
        );
        form.indexes.clear();
        form.add_foreign_key("public");
        assert!(form.blocker().is_some());
        assert!(form.statements("public").is_empty());
    }
}
