//! Filter bar completion: the grid's columns, replacing only the word under the cursor.

use gpui_kit::base::input::Rope;

use super::*;
use crate::model::DbKind;
use crate::ui::grid::completion::{filter_items, highlighter};

fn labels(clause: &str, text: &str) -> Vec<lsp_types::CompletionItem> {
    let (cols, _) = abc();
    let rope = Rope::from(text);
    let table = ("public", "users", DbKind::Postgres);
    filter_items(table, &cols, clause, &rope, text.len())
}

#[test]
fn where_completes_columns_over_the_partial_word() {
    let items = labels("WHERE", "a = 1 AND b");
    let b = items
        .iter()
        .find(|i| i.label == "b")
        .expect("column b offered");
    let Some(lsp_types::CompletionTextEdit::Edit(edit)) = &b.text_edit else {
        panic!("an edit");
    };
    assert_eq!(
        (edit.range.start.character, edit.range.end.character),
        (10, 11)
    );
}

#[test]
fn order_by_completes_columns_and_keywords() {
    let items = labels("ORDER BY", "c ");
    assert!(
        !items.iter().any(|i| i.label == "a"),
        "a column follows a comma, not a name"
    );
    assert!(items.iter().any(|i| i.label.eq_ignore_ascii_case("DESC")));
    let items = labels("ORDER BY", "c, ");
    assert!(items.iter().any(|i| i.label == "a"));
}

#[gpui_kit::test]
fn a_bare_filter_highlights_as_part_of_its_statement(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::AppContext as _;
    struct Names(std::sync::Mutex<Vec<String>>);
    impl gpui_kit::base::input::HighlightStyleResolver for Names {
        fn style(&self, name: &str) -> Option<gpui_kit::HighlightStyle> {
            self.0.lock().unwrap().push(name.to_string());
            Some(Default::default())
        }
    }
    let cx = cx.add_empty_window();
    let text = "a = 1 AND b = 'x'";
    let names = Names(Default::default());
    let styles = cx.update(|window, cx| {
        let editor = cx.new(|cx| gpui_kit::component::input::EditorState::new(window, cx));
        let mut h = highlighter("WHERE")("sql").expect("a highlighter");
        editor.update(cx, |_, cx| {
            h.update(None, &Rope::from(text), false, window, cx)
        });
        h.styles(&(0..text.len()), &names)
    });
    assert!(styles.iter().all(|(r, _)| r.end <= text.len()));
    let names = names.0.into_inner().unwrap();
    assert!(names.iter().any(|n| n == "string"), "{names:?}");
    assert!(names.iter().any(|n| n == "field"), "{names:?}");
}
