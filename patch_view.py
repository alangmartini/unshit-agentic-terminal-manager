import re

with open('crates/unshit-macros/src/view.rs') as f:
    content = f.read()

# 1. Add node_ref local var in parse
old = '        let mut memo = None;\n        let mut input_type = None;'
new = '        let mut memo = None;\n        let mut node_ref = None;\n        let mut input_type = None;'
assert old in content, "pattern 1 not found"
content = content.replace(old, new, 1)

# 2. Add node_ref parsing branch (before the else clause)
old = '                } else if attr_name == "value" {\n                    let attr_val: LitStr = content.parse()?;\n                    option_value = Some(attr_val);\n                } else {'
new = '                } else if attr_name == "value" {\n                    let attr_val: LitStr = content.parse()?;\n                    option_value = Some(attr_val);\n                } else if attr_name == "node_ref" {\n                    let expr: syn::Expr = content.parse()?;\n                    node_ref = Some(expr);\n                } else {'
assert old in content, "pattern 2 not found"
content = content.replace(old, new, 1)

# 3. Update error message to include node_ref
old = 'checked`, `min`, `max`, `step`, `name`, `selected`, `value'
new = 'checked`, `min`, `max`, `step`, `name`, `selected`, `value`, `node_ref'
assert old in content, "pattern 3 not found"
content = content.replace(old, new, 1)

# 4. Add node_ref to Ok(ViewNode { ... })
old = '            memo,\n            selected,'
new = '            memo,\n            node_ref,\n            selected,'
assert old in content, "pattern 4 not found"
content = content.replace(old, new, 1)

with open('crates/unshit-macros/src/view.rs', 'w') as f:
    f.write(content)
print('done')
