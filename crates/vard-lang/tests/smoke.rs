use vard_lang::compile;

fn diags(src: &str) -> String {
    let c = compile(&[("app.vard".to_string(), src.to_string())]);
    c.render_diags()
}

#[test]
fn std_is_clean() {
    let c = compile(&[]);
    assert!(c.diags.is_empty(), "{}", c.render_diags());
}

#[test]
fn todo_app_is_clean() {
    let src = r#"
struct Todo { id: Int, title: String, done: Bool = false }

view App {
  state todos: [Todo] = []
  state draft = ""
  state nextId = 1
  let left = todos.filter(t => !t.done).count

  fn add() {
    if draft.trim().isEmpty { return }
    todos.append(Todo(id: nextId, title: draft.trim()))
    nextId += 1
    draft = ""
  }

  Column(padding: 16, gap: 12) {
    Text("Todos", style: .title)
    Row(gap: 8) {
      Input(value: draft, placeholder: "What needs doing?", onSubmit: add, grow: 1)
      Button("Add", onTap: add)
    }
    for t in todos by t.id {
      Row(gap: 8, align: .center) {
        Checkbox(value: t.done, label: t.title, grow: 1)
        Button("Delete", kind: .text, onTap: { todos.remove(t) })
      }
    }
    if todos.isEmpty {
      Text("Nothing yet", color: .muted)
    } else {
      Text("{left} left", style: .caption, color: .muted)
    }
  }
}
"#;
    let d = diags(src);
    assert!(d.is_empty(), "{}", d);
}

#[test]
fn habits_get_fixes() {
    let src = r#"
import ui
view App {
  var count = 0
  VStack(spacing: 8, bg: .primry) {
    Text("Count: " + count, fontSize: 20)
    Button("Inc", onPress: { count++ })
    Text(count)
    if count.length() > 3 { Text("big") }
  }
}
"#;
    let c = compile(&[("app.vard".to_string(), src.to_string())]);
    let d = c.render_diags();
    println!("{}", d);
    for code in ["E0111", "E0112", "E0201", "E0202", "E0305", "E0303", "E0115", "E0301"] {
        assert!(d.contains(code), "missing {} in:\n{}", code, d);
    }
    let fixed = vard_lang::diag::apply_fixes(&c.sources, &c.diags);
    assert_eq!(fixed.len(), 1);
    println!("--- fixed ---\n{}", fixed[0].1);
    let c2 = compile(&[("app.vard".to_string(), fixed[0].1.clone())]);
    println!("--- after fix ---\n{}", c2.render_diags());
}
