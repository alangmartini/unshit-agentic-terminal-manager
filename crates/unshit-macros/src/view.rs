use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{braced, Ident, LitFloat, LitInt, LitStr, Result, Token};

struct ViewNode {
    tag: Ident,
    classes: Vec<Ident>,
    id: Option<LitStr>,
    key: Option<syn::Expr>,
    on_click: Option<syn::Expr>,
    on_resize: Option<syn::Expr>,
    on_change: Option<syn::Expr>,
    on_submit: Option<syn::Expr>,
    on_mount: Option<syn::Expr>,
    on_unmount: Option<syn::Expr>,
    placeholder: Option<LitStr>,
    captures_keyboard: Option<syn::Expr>,
    memo: Option<syn::Expr>,
    node_ref: Option<syn::Expr>,
    // select-specific
    selected: Option<LitInt>,
    // option-specific
    option_value: Option<LitStr>,
    text: Option<LitStr>,
    children: Vec<ViewNode>,
    // Input type attributes
    input_type: Option<LitStr>,
    checked: bool,
    min: Option<LitFloat>,
    max: Option<LitFloat>,
    step: Option<LitFloat>,
    name: Option<LitStr>,
}

impl Parse for ViewNode {
    fn parse(input: ParseStream) -> Result<Self> {
        let tag: Ident = input.parse()?;

        let mut classes = Vec::new();
        while input.peek(Token![.]) {
            input.parse::<Token![.]>()?;
            let class: Ident = input.parse()?;
            classes.push(class);
        }

        let mut id = None;
        let mut key = None;
        let mut on_click = None;
        let mut on_resize = None;
        let mut on_change = None;
        let mut on_submit = None;
        let mut on_mount = None;
        let mut on_unmount = None;
        let mut placeholder = None;
        let mut captures_keyboard = None;
        let mut memo = None;
        let mut node_ref = None;
        let mut input_type = None;
        let mut checked = false;
        let mut min = None;
        let mut max = None;
        let mut step = None;
        let mut name = None;
        let mut selected = None;
        let mut option_value = None;

        if input.peek(syn::token::Bracket) {
            let content;
            syn::bracketed!(content in input);
            while !content.is_empty() {
                let attr_name: Ident = content.parse()?;
                // `checked` is a boolean flag attribute (no `=value`).
                if attr_name == "checked" {
                    checked = true;
                    if content.peek(Token![,]) {
                        content.parse::<Token![,]>()?;
                    }
                    continue;
                }
                content.parse::<Token![=]>()?;
                if attr_name == "id" {
                    let attr_val: LitStr = content.parse()?;
                    id = Some(attr_val);
                } else if attr_name == "key" {
                    let expr: syn::Expr = content.parse()?;
                    key = Some(expr);
                } else if attr_name == "on_click" {
                    let expr: syn::Expr = content.parse()?;
                    on_click = Some(expr);
                } else if attr_name == "on_resize" {
                    let expr: syn::Expr = content.parse()?;
                    on_resize = Some(expr);
                } else if attr_name == "on_change" {
                    let expr: syn::Expr = content.parse()?;
                    on_change = Some(expr);
                } else if attr_name == "on_submit" {
                    let expr: syn::Expr = content.parse()?;
                    on_submit = Some(expr);
                } else if attr_name == "on_mount" {
                    let expr: syn::Expr = content.parse()?;
                    on_mount = Some(expr);
                } else if attr_name == "on_unmount" {
                    let expr: syn::Expr = content.parse()?;
                    on_unmount = Some(expr);
                } else if attr_name == "placeholder" {
                    let attr_val: LitStr = content.parse()?;
                    placeholder = Some(attr_val);
                } else if attr_name == "captures_keyboard" {
                    let expr: syn::Expr = content.parse()?;
                    captures_keyboard = Some(expr);
                } else if attr_name == "memo" {
                    let expr: syn::Expr = content.parse()?;
                    memo = Some(expr);
                } else if attr_name == "type" {
                    let attr_val: LitStr = content.parse()?;
                    input_type = Some(attr_val);
                } else if attr_name == "min" {
                    let attr_val: LitFloat = content.parse()?;
                    min = Some(attr_val);
                } else if attr_name == "max" {
                    let attr_val: LitFloat = content.parse()?;
                    max = Some(attr_val);
                } else if attr_name == "step" {
                    let attr_val: LitFloat = content.parse()?;
                    step = Some(attr_val);
                } else if attr_name == "name" {
                    let attr_val: LitStr = content.parse()?;
                    name = Some(attr_val);
                } else if attr_name == "selected" {
                    let lit: LitInt = content.parse()?;
                    selected = Some(lit);
                } else if attr_name == "value" {
                    let attr_val: LitStr = content.parse()?;
                    option_value = Some(attr_val);
                } else if attr_name == "node_ref" {
                    let expr: syn::Expr = content.parse()?;
                    node_ref = Some(expr);
                } else {
                    return Err(syn::Error::new(
                        attr_name.span(),
                        format!(
                            "unsupported attribute `{attr_name}`, \
                             supported: `id`, `key`, `on_click`, `on_resize`, \
                             `on_change`, `on_submit`, `on_mount`, `on_unmount`, \
                             `placeholder`, `captures_keyboard`, `memo`, `type`, \
                             `checked`, `min`, `max`, `step`, `name`, `selected`, `value`, \
                             `node_ref`"
                        ),
                    ));
                }
                if content.peek(Token![,]) {
                    content.parse::<Token![,]>()?;
                }
            }
        }

        let mut text = None;
        let mut children = Vec::new();

        if input.peek(syn::token::Brace) {
            let content;
            braced!(content in input);

            while !content.is_empty() {
                if content.peek(LitStr) {
                    text = Some(content.parse::<LitStr>()?);
                } else {
                    children.push(content.parse::<ViewNode>()?);
                }
            }
        }

        Ok(ViewNode {
            tag,
            classes,
            id,
            key,
            on_click,
            on_resize,
            on_change,
            on_submit,
            on_mount,
            on_unmount,
            placeholder,
            captures_keyboard,
            memo,
            node_ref,
            selected,
            option_value,
            text,
            children,
            input_type,
            checked,
            min,
            max,
            step,
            name,
        })
    }
}

fn tag_token(tag: &Ident) -> std::result::Result<TokenStream2, TokenStream> {
    let tag_str = tag.to_string();
    match tag_str.as_str() {
        "div" => Ok(quote! { ::unshit_core::element::Tag::Div }),
        "span" => Ok(quote! { ::unshit_core::element::Tag::Span }),
        "text" => Ok(quote! { ::unshit_core::element::Tag::Text }),
        "button" => Ok(quote! { ::unshit_core::element::Tag::Button }),
        "input" => Ok(quote! { ::unshit_core::element::Tag::Input }),
        "canvas" => Ok(quote! { ::unshit_core::element::Tag::Canvas }),
        "select" => Ok(quote! { ::unshit_core::element::Tag::Select }),
        "option" => Ok(quote! { ::unshit_core::element::Tag::Option }),
        _ => {
            let err =
                syn::Error::new(tag.span(), format!("unknown tag `{tag_str}`")).to_compile_error();
            Err(err.into())
        }
    }
}

fn generate_node(node: &ViewNode) -> std::result::Result<TokenStream2, TokenStream> {
    let tag_str = node.tag.to_string();

    // Special handling for select: collect option children into with_options()
    if tag_str == "select" {
        return generate_select_node(node);
    }

    let tag = tag_token(&node.tag)?;

    let class_calls: Vec<TokenStream2> = node
        .classes
        .iter()
        .map(|c| {
            let s = c.to_string();
            quote! { .with_class(#s) }
        })
        .collect();

    let id_call = node.id.as_ref().map(|id| {
        quote! { .with_id(#id) }
    });

    let key_call = node.key.as_ref().map(|key_expr| {
        quote! { .with_key(::std::string::ToString::to_string(&(#key_expr))) }
    });

    let on_click_call = node.on_click.as_ref().map(|expr| {
        quote! { .on_click(#expr) }
    });

    let on_resize_call = node.on_resize.as_ref().map(|expr| {
        quote! { .on_resize(#expr) }
    });

    let on_change_call = node.on_change.as_ref().map(|expr| {
        quote! { .on_change(#expr) }
    });

    let on_submit_call = node.on_submit.as_ref().map(|expr| {
        quote! { .on_submit(#expr) }
    });

    let on_mount_call = node.on_mount.as_ref().map(|expr| {
        quote! { .on_mount(#expr) }
    });

    let on_unmount_call = node.on_unmount.as_ref().map(|expr| {
        quote! { .on_unmount(#expr) }
    });

    let placeholder_call = node.placeholder.as_ref().map(|t| {
        quote! { .with_placeholder(#t) }
    });

    let captures_keyboard_call = node.captures_keyboard.as_ref().map(|expr| {
        quote! { .captures_keyboard(#expr) }
    });

    let memo_call = node.memo.as_ref().map(|memo_expr| {
        quote! {
            .with_memo_key({
                use ::std::hash::{Hash, Hasher};
                let mut hasher = ::std::collections::hash_map::DefaultHasher::new();
                (#memo_expr).hash(&mut hasher);
                hasher.finish()
            })
        }
    });

    let input_type_call = node.input_type.as_ref().map(|t| {
        let s = t.value();
        quote! {
            .with_input_type(::unshit_core::element::InputType::from_str(#s))
        }
    });

    let checked_call = if node.checked { Some(quote! { .with_checked(true) }) } else { None };

    let min_call = node.min.as_ref().map(|v| {
        quote! { .with_min(#v) }
    });

    let max_call = node.max.as_ref().map(|v| {
        quote! { .with_max(#v) }
    });

    let step_call = node.step.as_ref().map(|v| {
        quote! { .with_step(#v) }
    });

    let name_call = node.name.as_ref().map(|n| {
        quote! { .with_name(#n) }
    });

    let node_ref_call = node.node_ref.as_ref().map(|ref_expr| {
        quote! { .with_ref((#ref_expr).clone()) }
    });

    let text_call = node.text.as_ref().map(|t| {
        quote! { .with_text(#t) }
    });

    let child_calls: Vec<TokenStream2> = node
        .children
        .iter()
        .map(|child| {
            let child_gen = generate_node(child)?;
            Ok(quote! { .with_child(#child_gen) })
        })
        .collect::<std::result::Result<_, TokenStream>>()?;

    Ok(quote! {
        ::unshit_core::element::ElementDef::new(#tag)
            #(#class_calls)*
            #id_call
            #key_call
            #on_click_call
            #on_resize_call
            #on_change_call
            #on_submit_call
            #on_mount_call
            #on_unmount_call
            #placeholder_call
            #captures_keyboard_call
            #memo_call
            #input_type_call
            #checked_call
            #min_call
            #max_call
            #step_call
            #name_call
            #node_ref_call
            #text_call
            #(#child_calls)*
    })
}

/// Generate code for a `select` node: collect option children into with_options().
fn generate_select_node(node: &ViewNode) -> std::result::Result<TokenStream2, TokenStream> {
    let tag = quote! { ::unshit_core::element::Tag::Select };

    let class_calls: Vec<TokenStream2> = node
        .classes
        .iter()
        .map(|c| {
            let s = c.to_string();
            quote! { .with_class(#s) }
        })
        .collect();

    let id_call = node.id.as_ref().map(|id| {
        quote! { .with_id(#id) }
    });

    let on_change_call = node.on_change.as_ref().map(|expr| {
        quote! { .on_change(#expr) }
    });

    let on_click_call = node.on_click.as_ref().map(|expr| {
        quote! { .on_click(#expr) }
    });

    // Collect option children into a Vec<(String, String)>
    let mut option_entries: Vec<TokenStream2> = Vec::new();
    for child in &node.children {
        if child.tag != "option" {
            let err = syn::Error::new(child.tag.span(), "select children must be option elements")
                .to_compile_error();
            return Err(err.into());
        }
        let value = match &child.option_value {
            Some(v) => v.value(),
            None => String::new(),
        };
        let label = child.text.as_ref().map(LitStr::value).unwrap_or_default();
        option_entries.push(quote! {
            (::std::string::String::from(#value), ::std::string::String::from(#label))
        });
    }

    let selected_call = node.selected.as_ref().map(|idx| {
        quote! { .with_selected_index(#idx) }
    });

    Ok(quote! {
        ::unshit_core::element::ElementDef::new(#tag)
            #(#class_calls)*
            #id_call
            #on_change_call
            #on_click_call
            #selected_call
            .with_options(::std::vec![#(#option_entries),*])
    })
}

pub fn parse_view(input: TokenStream) -> TokenStream {
    let node = syn::parse_macro_input!(input as ViewNode);
    let generated = match generate_node(&node) {
        Ok(ts) => ts,
        Err(err) => return err,
    };

    quote! {
        ::unshit_core::element::ElementTree {
            root: #generated,
        }
    }
    .into()
}
