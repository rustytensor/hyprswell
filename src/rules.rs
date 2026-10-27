use serde::Deserialize;

#[derive(Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Workspace {
    pub id: i64,
    pub name: String,
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Client {
    pub address: String,
    pub class: String,
    pub title: String,
    pub workspace: Workspace,
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Monitor {
    pub name: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DockItem {
    pub id: String,
    pub windows: Vec<Client>,
}

pub fn items(pins: &[String], clients: &[Client]) -> Vec<DockItem> {
    let mut result = vec![DockItem { id: "launcher".into(), windows: vec![] }];
    for id in pins {
        if !result.iter().any(|item| item.id == *id) {
            result.push(DockItem { id: id.clone(), windows: vec![] });
        }
    }
    let mut sorted: Vec<_> = clients.iter().filter(|c| !c.class.is_empty()).collect();
    sorted.sort_by(|a, b| a.workspace.id.cmp(&b.workspace.id).then(a.class.cmp(&b.class)));
    for client in sorted {
        if let Some(item) = result.iter_mut().skip(1).find(|item| item.id == client.class) {
            item.windows.push(client.clone());
        } else {
            result.push(DockItem { id: client.class.clone(), windows: vec![client.clone()] });
        }
    }
    result
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Launch(String),
    Focus(String),
    Minimize(String),
    Restore(String),
    Menu(Vec<Client>),
}

fn minimized(c: &Client) -> bool {
    c.workspace.name == "special:minimized"
}

pub fn left_click(id: &str, windows: &[Client], active: &str) -> Action {
    match windows {
        [] => Action::Launch(id.into()),
        [c] if minimized(c) => Action::Restore(c.address.clone()),
        [c] if c.address == active => Action::Minimize(c.address.clone()),
        [c] => Action::Focus(c.address.clone()),
        _ => Action::Menu(windows.to_vec()),
    }
}

pub fn menu_pick(c: &Client) -> Action {
    if minimized(c) { Action::Restore(c.address.clone()) } else { Action::Focus(c.address.clone()) }
}

pub fn window_line(c: &Client) -> String {
    let title = if c.title.chars().count() > 48 {
        c.title.chars().take(47).collect::<String>() + "…"
    } else {
        c.title.clone()
    };
    let workspace = if minimized(c) { "minimized" } else { &c.workspace.name };
    format!("{title}  · {workspace}")
}

pub fn hover_text(name: &str, windows: &[Client]) -> String {
    std::iter::once(name.to_owned()).chain(windows.iter().map(window_line)).collect::<Vec<_>>().join("\n")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Indicator { None, One, Two, Hollow }

pub fn indicator(windows: &[Client]) -> Indicator {
    if windows.is_empty() { Indicator::None }
    else if windows.iter().all(minimized) { Indicator::Hollow }
    else if windows.len() == 1 { Indicator::One }
    else { Indicator::Two }
}

pub fn icon_size(text: &str) -> u32 {
    text.lines().filter_map(|line| line.strip_prefix("ICON_SIZE="))
        .filter_map(|value| value.trim().parse::<u32>().ok()).find(|&size| size > 0).unwrap_or(44)
}

// Count apps only, not the launcher; retain the old three-app integer steps.
pub fn drawn_size(size: u32, count: usize) -> u32 {
    let drawn = u64::from(size) * 5 / 4;
    let divisor = 6 + count.saturating_sub(6) / 3;
    (drawn * 6 / divisor as u64).min(u64::from(u32::MAX)) as u32
}

pub fn select_monitors(monitors: &[Monitor], output: Option<&str>) -> Vec<String> {
    monitors.iter().filter(|m| match output {
        Some(name) => m.name == name,
        None => m.description.contains("S34C65xU"),
    }).map(|m| m.name.clone()).collect()
}
