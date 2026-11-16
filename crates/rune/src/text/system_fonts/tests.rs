use super::*;

#[test]
fn bare_emoji_requests_colour_without_generic_discovery() {
    let mut ts = TextSystem::default();
    ts.fallbacks.symbols = true;
    ts.fallbacks.asked.insert('😀');
    assert!(ts.emoji.is_none());
    assert!(!ts.fallbacks.asked.contains(&'\u{FE0F}'));
    ts.load_fallbacks("😀");
    assert!(ts.fallbacks.asked.contains(&'\u{FE0F}'), "bare emoji must request the colour family even without missing-glyph discovery");
}

#[test]
fn absent_colour_family_keeps_text_fallback_without_discovery() {
    let samples = ["a😀b", "a✅ ⚡b", "a❤ ☀ ✔ ☺ ©b", "a❤\u{FE0F}b", "a😀\u{FE0E}b"];
    let mut ts = TextSystem::default();
    ts.fallbacks.symbols = true;
    ts.fallbacks.asked.extend(samples.iter().flat_map(|s| s.chars()));
    ts.math = Some(Vec::new());
    let families: Vec<_> = ts.font_cx.collection.family_ids().collect();
    let style = TextStyle::new(13.0, 400.0);
    let latin = ts.measure("Hello", style, 2.0);
    for _ in 0..2 {
        for sample in samples {
            assert_eq!(super::super::tests::colour(&mut ts, sample, &[], style), 0, "{sample:?}");
            assert!(ts.measure(sample, style, 2.0).0 > 0.0);
        }
        assert!(ts.emoji.is_none() && ts.symbol_families.is_empty());
        assert!(ts.math.as_ref().unwrap().is_empty());
        assert_eq!(ts.font_cx.collection.family_ids().collect::<Vec<_>>(), families);
        assert!(ts.font_files.is_empty());
        assert_eq!(ts.measure("Hello", style, 2.0), latin);
    }
}
