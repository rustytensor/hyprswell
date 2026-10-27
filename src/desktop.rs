use gio::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;

#[derive(Default)]
struct Cache {
    ids: HashMap<String, Option<gio::DesktopAppInfo>>,
    classes: Option<HashMap<String, gio::DesktopAppInfo>>,
}

thread_local! { static CACHE: RefCell<Cache> = RefCell::new(Cache::default()); }

// GIO's monitor tells the main thread when installed desktop entries change.
pub fn invalidate() {
    CACHE.with(|cache| *cache.borrow_mut() = Cache::default());
}

pub fn app_info(id: &str) -> Option<gio::DesktopAppInfo> {
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(info) = cache.ids.get(id) { return info.clone(); }
        let mut found = None;
        for candidate in [id.to_owned(), id.to_lowercase()] {
            if let Some(info) = gio::DesktopAppInfo::new(&format!("{candidate}.desktop")) {
                found = Some(info);
                break;
            }
        }
        if found.is_none() {
            let classes = cache.classes.get_or_insert_with(|| {
                let mut classes = HashMap::new();
                for info in gio::AppInfo::all().into_iter()
                    .filter_map(|info| info.downcast::<gio::DesktopAppInfo>().ok()) {
                    if let Some(class) = info.startup_wm_class() {
                        classes.entry(class.to_lowercase()).or_insert(info);
                    }
                }
                classes
            });
            found = classes.get(&id.to_lowercase()).cloned();
        }
        cache.ids.insert(id.to_owned(), found.clone());
        found
    })
}
