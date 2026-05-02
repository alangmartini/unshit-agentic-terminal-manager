use unshit_core::style::parse::CompiledStylesheet;

/// Repro: when a CSS comment precedes a rule, the comment text can leak
/// into the selector of the following rule. This test checks that a
/// single `/* foo */` comment right before `.nav { ... }` still gives us
/// the bare `.nav` selector, not something like `/* nav */ .nav`.
#[test]
fn comment_before_rule_does_not_leak_into_selector() {
    let css = r#"
/* nav */
.nav {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 22px 0;
    width: 1360px;
}
"#;
    let ss = CompiledStylesheet::parse(css);
    for rule in &ss.rules {
        eprintln!("rule selector={:?} decls={}", rule.selector, rule.declarations.len());
    }

    let nav_rule = ss
        .rules
        .iter()
        .find(|r| {
            // A plain `.nav` rule has exactly one compound selector,
            // containing exactly one part: `Class("nav")`.
            r.selector.parts.len() == 1
                && r.selector.parts[0].0.len() == 1
                && matches!(
                    &r.selector.parts[0].0[0],
                    unshit_core::style::parse::SelectorPart::Class(c) if c == "nav"
                )
        })
        .expect("expected a plain `.nav` rule");
    assert!(
        nav_rule.declarations.len() >= 5,
        "plain .nav rule should have >=5 decls, got {}",
        nav_rule.declarations.len()
    );
}

/// When a comment appears BETWEEN two rules, not before the first one,
/// the parser's position is different (it starts after the previous `}`)
/// but the comment must still not leak into the selector.
#[test]
fn comment_between_rules_does_not_leak_into_selector() {
    let css = r#"
.first {
    color: red;
}
/* nav */
.nav {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 22px 0;
    width: 1360px;
}
"#;
    let ss = CompiledStylesheet::parse(css);
    for rule in &ss.rules {
        eprintln!("rule selector={:?} decls={}", rule.selector, rule.declarations.len());
    }

    let nav_rule = ss
        .rules
        .iter()
        .find(|r| {
            r.selector.parts.len() == 1
                && r.selector.parts[0].0.len() == 1
                && matches!(
                    &r.selector.parts[0].0[0],
                    unshit_core::style::parse::SelectorPart::Class(c) if c == "nav"
                )
        })
        .expect("expected a plain `.nav` rule");
    assert!(
        nav_rule.declarations.len() >= 5,
        "plain .nav rule should have >=5 decls even after a preceding comment, got {}",
        nav_rule.declarations.len()
    );
}
