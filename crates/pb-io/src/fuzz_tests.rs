//! Readers of outside data never panic, whatever they are given.

use proptest::prelude::*;

/// Mermaid-ish text: ids from a small set, so edges meet, loops and chains included.
fn flowchart() -> impl Strategy<Value = String> {
    let id = prop::sample::select(vec!["A", "B", "C", "D", "E", "F"]);
    let shape = prop::sample::select(vec!["", "[box]", "(round)", "{yes?}", "((o))", "[(db)]"]);
    let arrow = prop::sample::select(vec!["-->", "---", "-.->", "==>", "-->|label|", "--"]);
    let line = (id.clone(), shape.clone(), arrow, id, shape)
        .prop_map(|(a, sa, arrow, b, sb)| format!("    {a}{sa} {arrow} {b}{sb}"));
    let header = prop::sample::select(vec!["flowchart TD", "graph LR", "flowchart BT", "x"]);
    (header, prop::collection::vec(line, 0..12))
        .prop_map(|(header, lines)| format!("{header}\n{}", lines.join("\n")))
}

proptest! {
    #[test]
    fn csv_reads_anything(text in ".*", tabbed in "[a-z\t\"\r\n,;]*") {
        let rows = crate::csv::parse(&text);
        prop_assert!(rows.iter().all(|r| r.len() == rows[0].len()));
        let rows = crate::csv::parse_tsv(&tabbed);
        prop_assert!(rows.iter().all(|r| r.len() == rows[0].len()));
    }

    #[test]
    fn mermaid_reads_and_lays_out_anything(text in flowchart(), noise in ".*") {
        for source in [text, noise] {
            if let Ok(chart) = crate::mermaid::parse(&source) {
                let sizes = vec![pb_geom::kurbo::Size::new(120.0, 60.0); chart.nodes.len()];
                let places = crate::mermaid::layout(&chart, &sizes);
                prop_assert_eq!(places.len(), chart.nodes.len());
                prop_assert!(places.iter().all(|p| p.x.is_finite() && p.y.is_finite()));
            }
        }
    }

    #[test]
    fn page_previews_read_any_html(html in ".*", tags in "(<[a-z]+( [a-z]+=\"[^\"]*\")?>|[^<]){0,40}") {
        crate::preview::parse(&html);
        crate::preview::parse(&tags);
    }

    #[test]
    fn broken_pictures_and_documents_are_errors(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        let _ = crate::image::dimensions(&bytes);
        let _ = crate::image::decode(&bytes, 256);
        let _ = crate::pdf::page_count(&bytes);
    }
}
