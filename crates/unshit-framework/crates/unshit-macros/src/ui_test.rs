use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{parse_macro_input, Expr, ExprLit, ItemFn, Lit, Meta};

/// Configuration parsed from `#[ui_test(...)]` attributes.
#[derive(Default)]
struct UiTestConfig {
    headed: Option<bool>,
    slow_mo: Option<u64>,
    width: Option<u32>,
    height: Option<u32>,
    gpu: Option<bool>,
    timeout: Option<u64>,
}

impl UiTestConfig {
    /// Collect all configured env vars as (name, value) pairs.
    fn env_vars(&self) -> Vec<(&str, String)> {
        let mut vars = Vec::new();
        if let Some(v) = self.headed {
            vars.push(("UNSHIT_TEST_HEADED", bool_to_env(v)));
        }
        if let Some(v) = self.slow_mo {
            vars.push(("UNSHIT_TEST_SLOW_MO", v.to_string()));
        }
        if let Some(v) = self.width {
            vars.push(("UNSHIT_TEST_WIDTH", v.to_string()));
        }
        if let Some(v) = self.height {
            vars.push(("UNSHIT_TEST_HEIGHT", v.to_string()));
        }
        if let Some(v) = self.gpu {
            vars.push(("UNSHIT_TEST_GPU", bool_to_env(v)));
        }
        if let Some(v) = self.timeout {
            vars.push(("UNSHIT_TEST_TIMEOUT", v.to_string()));
        }
        vars
    }
}

fn bool_to_env(b: bool) -> String {
    if b { "1" } else { "0" }.to_owned()
}

pub fn ui_test_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input_fn = parse_macro_input!(item as ItemFn);
    let config = parse_config(attr);

    let fn_name = &input_fn.sig.ident;
    let fn_block = &input_fn.block;
    let fn_vis = &input_fn.vis;
    let fn_attrs = &input_fn.attrs;

    let env_vars = config.env_vars();

    let env_setup: Vec<TokenStream2> =
        env_vars.iter().map(|(name, val)| quote! { std::env::set_var(#name, #val); }).collect();

    let env_cleanup: Vec<TokenStream2> =
        env_vars.iter().map(|(name, _)| quote! { std::env::remove_var(#name); }).collect();

    let expanded = quote! {
        #(#fn_attrs)*
        #[test]
        #fn_vis fn #fn_name() {
            #(#env_setup)*

            let __ui_test_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                #fn_block
            }));

            #(#env_cleanup)*

            if let Err(err) = __ui_test_result {
                std::panic::resume_unwind(err);
            }
        }
    };

    expanded.into()
}

fn parse_config(attr: TokenStream) -> UiTestConfig {
    let mut config = UiTestConfig::default();

    if attr.is_empty() {
        return config;
    }

    let parser = syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated;
    let metas = match syn::parse::Parser::parse(parser, attr) {
        Ok(m) => m,
        Err(e) => {
            panic!("Failed to parse ui_test attributes: {}", e);
        }
    };

    for meta in metas {
        match &meta {
            Meta::NameValue(nv) => {
                let name = nv.path.get_ident().map(|i| i.to_string()).unwrap_or_default();
                match name.as_str() {
                    "headed" => config.headed = Some(expr_to_bool(&nv.value)),
                    "slow_mo" => config.slow_mo = Some(expr_to_u64(&nv.value)),
                    "width" => config.width = Some(expr_to_u64(&nv.value) as u32),
                    "height" => config.height = Some(expr_to_u64(&nv.value) as u32),
                    "gpu" => config.gpu = Some(expr_to_bool(&nv.value)),
                    "timeout" => config.timeout = Some(expr_to_u64(&nv.value)),
                    other => panic!("Unknown ui_test attribute: `{}`", other),
                }
            }
            Meta::Path(p) => {
                let name = p.get_ident().map(|i| i.to_string()).unwrap_or_default();
                match name.as_str() {
                    "headed" => config.headed = Some(true),
                    "gpu" => config.gpu = Some(true),
                    other => panic!("Unknown ui_test flag: `{}`", other),
                }
            }
            _ => panic!("Unsupported attribute format in ui_test"),
        }
    }

    config
}

fn expr_to_bool(expr: &Expr) -> bool {
    match expr {
        Expr::Lit(ExprLit { lit: Lit::Bool(b), .. }) => b.value,
        Expr::Lit(ExprLit { lit: Lit::Int(i), .. }) => i.base10_parse::<u64>().unwrap_or(0) != 0,
        _ => panic!("Expected a boolean or integer literal"),
    }
}

fn expr_to_u64(expr: &Expr) -> u64 {
    match expr {
        Expr::Lit(ExprLit { lit: Lit::Int(i), .. }) => {
            i.base10_parse::<u64>().expect("Expected an integer literal")
        }
        _ => panic!("Expected an integer literal"),
    }
}
