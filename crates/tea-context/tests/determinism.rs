use crate::common;

use common::{module, segment};
use tea_context::{BudgetBehavior, PromptAuthority, PromptBudget, PromptCompiler};

#[test]
fn generated_module_rotations_compile_byte_identically() {
    let modules = (0..12)
        .map(|index| {
            module(
                &format!("module.{index:02}"),
                match index % 4 {
                    0 => PromptAuthority::Organization,
                    1 => PromptAuthority::Product,
                    2 => PromptAuthority::Workspace,
                    _ => PromptAuthority::Session,
                },
                (index % 3) as i16,
                vec![segment(
                    &format!("segment.{index:02}"),
                    &format!("content-{index:02}"),
                    if index % 5 == 0 {
                        BudgetBehavior::Omit
                    } else {
                        BudgetBehavior::Required
                    },
                )],
            )
        })
        .collect::<Vec<_>>();
    let expected = PromptCompiler
        .compile(modules.clone(), PromptBudget::new(4096, 4096).unwrap())
        .unwrap();
    for rotation in 0..modules.len() {
        let mut input = modules.clone();
        input.rotate_left(rotation);
        let actual = PromptCompiler
            .compile(input, PromptBudget::new(4096, 4096).unwrap())
            .unwrap();
        assert_eq!(actual, expected, "rotation {rotation}");
    }
}

#[test]
fn deterministic_budget_boundary_never_splits_utf8() {
    for max_bytes in 1..64 {
        let modules = [module(
            "unicode",
            PromptAuthority::Workspace,
            0,
            vec![
                segment("required", "x", BudgetBehavior::Required),
                segment(
                    "optional",
                    "🙂你好-abcdefghijklmnopqrstuvwxyz",
                    BudgetBehavior::Truncate,
                ),
            ],
        )];
        let result = PromptCompiler.compile(modules, PromptBudget::new(max_bytes, 1024).unwrap());
        if let Ok(prompt) = result {
            assert!(std::str::from_utf8(prompt.text().as_bytes()).is_ok());
            assert!(prompt.bytes() <= max_bytes);
        }
    }
}
