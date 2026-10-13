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
    /// Each UNIQUE constraint's name and its columns' positions in `columns`, so a rename can't
    /// move a key onto another column.
    pub unique_keys: Vec<(String, Vec<usize>)>,
}

impl TableForm {
    pub fn modify(before: TableDef) -> Self {
        let columns = before
            .columns
            .iter()
            .map(|c| (Some(c.name.clone()), c.clone()))
            .collect();
        let position = |name: &String| before.columns.iter().position(|c| &c.name == name);
        let unique_keys = before
            .unique_keys
            .iter()
            .map(|u| {
                (
                    u.name.clone(),
                    u.columns.iter().filter_map(position).collect(),
                )
            })
            .collect();
        Self {
            name: before.name.clone(),
            indexes: before.indexes.clone(),
            foreign_keys: before.foreign_keys.clone(),
            unique_keys,
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

    /// Column `ix` has a single-column UNIQUE constraint.
    pub fn is_unique(&self, ix: usize) -> bool {
        self.unique_keys.iter().any(|(_, cols)| cols == &[ix])
    }

    /// Adds `<table>_<col>_key`, or removes every single-column constraint on the column.
    pub fn set_unique(&mut self, ix: usize, unique: bool) {
        if !unique {
            self.unique_keys.retain(|(_, cols)| cols != &[ix]);
            return;
        }
        if !self.is_unique(ix) {
            let name = format!("{}_{}_key", self.name, self.columns[ix].1.name);
            self.unique_keys.push((name, vec![ix]));
        }
    }

    /// Removes column `ix` and every UNIQUE constraint on it; Postgres drops those with it.
    pub fn remove_column(&mut self, ix: usize) {
        self.columns.remove(ix);
        self.unique_keys.retain(|(_, cols)| !cols.contains(&ix));
        for col in self.unique_keys.iter_mut().flat_map(|(_, cols)| cols) {
            *col -= (*col > ix) as usize;
        }
    }

    /// Postgres requires a PK column to be not null, so ticking it ticks that too.
    pub fn set_pk(&mut self, ix: usize, is_pk: bool) {
        let c = &mut self.columns[ix].1;
        c.is_pk = is_pk;
        c.nullable &= !is_pk;
    }

    /// The UNIQUE constraints, by their columns' current names.
    pub fn unique_indexes(&self) -> Vec<Index> {
        let name = |&ix: &usize| self.columns[ix].1.name.clone();
        self.unique_keys
            .iter()
            .map(|(n, cols)| Index {
                name: n.clone(),
                columns: cols.iter().map(name).collect(),
                unique: true,
            })
            .collect()
    }

    /// The positions in `unique_keys` of the multi-column constraints, listed on the Indexes tab.
    pub fn listed_uniques(&self) -> Vec<usize> {
        (0..self.unique_keys.len())
            .filter(|&i| self.unique_keys[i].1.len() > 1)
            .collect()
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
                unique_keys: self.unique_indexes(),
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
    fn unique_keys_stay_on_their_column_through_renames_and_unticking_drops_every_single_column_key()
     {
        let mut form = TableForm::create();
        form.name = "t".into();
        form.columns[0].1.name = "a".into();
        form.set_unique(0, true);
        form.set_unique(0, true);
        assert_eq!(form.unique_keys, [("t_a_key".to_string(), vec![0])]);
        form.add_column();
        form.columns[1].1.name = "b".into();
        form.set_unique(1, true);
        // Renaming a through b's name and on leaves b's key on b.
        form.columns[0].1.name = "b".into();
        form.columns[0].1.name = "b2".into();
        let columns: Vec<_> = form
            .unique_indexes()
            .into_iter()
            .map(|u| u.columns)
            .collect();
        assert_eq!(columns, [["b2"], ["b"]]);
        form.unique_keys.push(form.unique_keys[0].clone());
        form.set_unique(0, false);
        assert_eq!(form.unique_keys, [("t_b_key".to_string(), vec![1])]);
        form.remove_column(0);
        assert_eq!(form.unique_keys, [("t_b_key".to_string(), vec![0])]);
        form.set_pk(0, true);
        assert!(!form.columns[0].1.nullable);
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
