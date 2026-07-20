use oxc_allocator::Allocator;
use oxc_formatter::{JsFormatOptions, format};
use oxc_span::SourceType;

fn format_ets(source_text: &str) -> String {
    let allocator = Allocator::default();
    format(&allocator, source_text, SourceType::ets(), JsFormatOptions::default(), None)
        .unwrap()
        .print()
        .unwrap()
        .into_code()
}

#[test]
fn arkui_component_chain_comment_stays_in_place_after_reformat() {
    let source = r"struct S {
  build() {
    Row() {}
      .width(100)
      //.disabled(true)
      .height(200)
  }
}
";

    let first = format_ets(source);
    let second = format_ets(&first);

    let width = second.find(".width(100)").expect("width chain call");
    let comment = second.find("//.disabled(true)").expect("disabled chain comment");
    let height = second.find(".height(200)").expect("height chain call");

    assert!(
        width < comment && comment < height,
        "ArkUI component chain comment moved after reformat:\n{second}"
    );
}
