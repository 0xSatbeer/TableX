use crate::ddl::{TableEdit, alter_table};
use crate::model::{Column, TableDef};

/// The table editor's state, apart from its inputs: the definition it opened on and the edits.
#[derive(Clone, Debug, PartialEq)]
pub struct TableForm {
    pub before: TableDef,
    pub name: String,
    /// Each column with its original name, `None` when added, so a rename stays a rename.
    pub columns: Vec<(Option<String>, Column)>,
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
            before,
            columns,
        }
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

    /// The statements the edits need; none while a name or a type is blank.
    pub fn statements(&self, schema: &str) -> Vec<String> {
        let blank = self
            .columns
            .iter()
            .any(|(_, c)| c.name.is_empty() || c.data_type.is_empty());
        if self.name.is_empty() || blank {
            return Vec::new();
        }
        alter_table(schema, &self.before, &self.edit())
    }

    fn edit(&self) -> TableEdit {
        let (origins, columns) = self.columns.iter().cloned().unzip();
        TableEdit {
            def: TableDef {
                name: self.name.clone(),
                columns,
                indexes: self.before.indexes.clone(),
                foreign_keys: self.before.foreign_keys.clone(),
            },
            origins,
        }
    }
}
