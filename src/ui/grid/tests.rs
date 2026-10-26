use super::{
    BigEvents, Block, CellKind, ColumnMeta, Editor, EditorKind, Grid, GridSource, HeaderModel,
    MAX_CHARS, Page, SortAvail, Value, aligns_right, band_cols, cmp_values, copy_cols,
    copy_in_list, copy_ranges, csv, display_widths, editor_kind, format_cell, grouped,
    gutter_digits, gutter_width, header_height, header_model, inspect_text, is_numeric_type, json,
    markdown, mock, move_col, nav, page_count, parse_tsv, parse_value, paste_targets, pick,
    prefix_sums, pretty_json, px, ranges_in_order, row_height, selection_after_drop,
    sort_availability, sql_in_list, sql_insert, timestamp_value, tsv, visible_cols, wall_time,
};

mod add_delete;
mod cell_edit;
mod columns;
mod copy;
mod editor;
mod export;
mod format;
mod header;
mod live_pages;
mod submit;
mod table_page;
mod values;

/// The open in-cell text input.
fn text_input(g: &super::Grid) -> gpui_kit::Entity<gpui_kit::component::input::InputState> {
    match &g.editing.as_ref().expect("an editor is open").editor {
        super::Editor::Text(input) => input.clone(),
        _ => panic!("expected the text editor"),
    }
}

fn abc() -> (Vec<ColumnMeta>, Vec<Vec<Value>>) {
    let cols = ["a", "b", "c"]
        .map(|n| ColumnMeta {
            name: n.into(),
            data_type: "text".into(),
            is_pk: false,
            ..Default::default()
        })
        .to_vec();
    (
        cols,
        vec![["a0", "b0", "c0"].map(|v| Value::Text(v.into())).to_vec()],
    )
}

/// What `Grid::copy_as` emits for display positions `display` of row 0.
fn copy(
    format: fn(&[ColumnMeta], &[Vec<Value>]) -> String,
    order: &[usize],
    hidden: &[bool],
    display: std::ops::Range<usize>,
) -> String {
    let (cols, rows) = abc();
    let ix = copy_cols(order, hidden, display);
    format(
        &pick(&cols, &ix),
        &rows.iter().map(|r| pick(r, &ix)).collect::<Vec<_>>(),
    )
}

fn col(name: &str, ty: &str, pk: bool) -> ColumnMeta {
    ColumnMeta {
        name: name.into(),
        data_type: ty.into(),
        is_pk: pk,
        ..Default::default()
    }
}

/// Row r, column c holds "v{r}{c}"; columns a, b, c.
fn vgrid() -> (Vec<ColumnMeta>, impl Fn(usize) -> Vec<Value>) {
    let cols = ["a", "b", "c"]
        .map(|n| ColumnMeta {
            name: n.into(),
            data_type: "text".into(),
            is_pk: false,
            ..Default::default()
        })
        .to_vec();
    (cols, |r| {
        (0..3)
            .map(|c| Value::Text(format!("v{r}{c}").into()))
            .collect()
    })
}
