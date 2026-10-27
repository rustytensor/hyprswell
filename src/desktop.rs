use gio::prelude::*;

pub fn app_info(id: &str) -> Option<gio::DesktopAppInfo> {
    for candidate in [id.to_owned(), id.to_lowercase()] {
        if let Some(info) = gio::DesktopAppInfo::new(&format!("{candidate}.desktop")) {
            return Some(info);
        }
    }
    let class = id.to_lowercase();
    gio::AppInfo::all().into_iter()
        .filter_map(|info| info.downcast::<gio::DesktopAppInfo>().ok())
        .find(|info| info.startup_wm_class().is_some_and(|wm| wm.to_lowercase() == class))
}
