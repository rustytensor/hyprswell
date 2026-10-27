use gtk::prelude::*;
use gtk::subclass::prelude::*;
use hyprswell::{motion, rules};
use std::cell::RefCell;

#[derive(Clone)]
pub struct Icon {
    pub paintable: gtk::gdk::Paintable,
    pub indicator: rules::Indicator,
    pub active: bool,
    pub shadow: bool,
    pub bounce: f64,
}

#[derive(Clone)]
pub struct Scene {
    pub width: i32,
    pub height: i32,
    pub layout: motion::Layout,
    pub icons: Vec<Icon>,
    pub indicator_y: f64,
}

mod imp {
    use super::*;
    #[derive(Default)]
    pub struct DockView {
        pub scene: RefCell<Option<Scene>>,
        pub bar_style: RefCell<Option<gtk::Box>>,
        pub indicator_style: RefCell<Option<gtk::Label>>,
        pub active_style: RefCell<Option<gtk::Box>>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for DockView {
        const NAME: &'static str = "HyprswellDockView";
        type Type = super::DockView;
        type ParentType = gtk::Widget;
    }
    impl ObjectImpl for DockView {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() { child.unparent(); }
        }
    }
    impl WidgetImpl for DockView {
        fn measure(&self, direction: gtk::Orientation, _: i32) -> (i32, i32, i32, i32) {
            let scene = self.scene.borrow();
            let size = scene.as_ref().map_or(1, |scene| {
                if direction == gtk::Orientation::Horizontal { scene.width } else { scene.height }
            });
            (size, size, -1, -1)
        }
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let scene = self.scene.borrow();
            let Some(scene) = scene.as_ref() else { return; };
            let styles = self.bar_style.borrow();
            let bar = styles.as_ref().unwrap();
            let rect = scene.layout.bar;
            snapshot.render_background(&bar.style_context(), rect.x, rect.y, rect.width, rect.height);
            snapshot.render_frame(&bar.style_context(), rect.x, rect.y, rect.width, rect.height);
            for (icon, rect) in scene.icons.iter().zip(&scene.layout.icons) {
                snapshot.save();
                if icon.shadow {
                    snapshot.push_shadow(&[gtk::gsk::Shadow::new(
                        gtk::gdk::RGBA::new(0.0, 0.0, 0.0, 0.35), 0.0, 6.0, 10.0)]);
                }
                snapshot.translate(&gtk::graphene::Point::new(rect.x as f32, (rect.y - icon.bounce) as f32));
                if let Some(symbolic) = icon.paintable.dynamic_cast_ref::<gtk::SymbolicPaintable>() {
                    symbolic.snapshot_symbolic(snapshot, rect.width, rect.height, &[bar.color(); 4]);
                } else { icon.paintable.snapshot(snapshot, rect.width, rect.height); }
                if icon.shadow { snapshot.pop(); }
                snapshot.restore();
            }
            let style = self.indicator_style.borrow();
            let color = style.as_ref().unwrap().color();
            let cr = snapshot.append_cairo(&gtk::graphene::Rect::new(0.0, 0.0, scene.width as f32, scene.height as f32));
            cr.set_source_rgba(color.red() as f64, color.green() as f64, color.blue() as f64, color.alpha() as f64);
            for (icon, rect) in scene.icons.iter().zip(&scene.layout.icons) {
                let centre = rect.x + rect.width / 2.0;
                let offsets: &[f64] = match icon.indicator {
                    rules::Indicator::None => &[], rules::Indicator::Two => &[-3.0, 3.0], _ => &[0.0],
                };
                for offset in offsets {
                    cr.new_sub_path();
                    cr.arc(centre + offset, scene.indicator_y, 2.0, 0.0, std::f64::consts::TAU);
                    if icon.indicator == rules::Indicator::Hollow { cr.set_line_width(1.2); let _ = cr.stroke(); }
                    else { let _ = cr.fill(); }
                }
                if icon.active {
                    let style = self.active_style.borrow();
                    snapshot.render_frame(&style.as_ref().unwrap().style_context(),
                        centre - 10.0, scene.indicator_y + 4.0, 20.0, 3.0);
                }
            }
        }
    }
}

glib::wrapper! {
    pub struct DockView(ObjectSubclass<imp::DockView>) @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl DockView {
    pub fn new() -> Self {
        let view: Self = glib::Object::builder().build();
        view.set_focusable(false);
        view.set_overflow(gtk::Overflow::Visible);
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        bar.add_css_class("dock-bar");
        let indicator = gtk::Label::new(None);
        indicator.add_css_class("dock-indicator");
        *view.imp().bar_style.borrow_mut() = Some(bar);
        *view.imp().indicator_style.borrow_mut() = Some(indicator);
        let active = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        active.add_css_class("dock-item");
        active.add_css_class("active");
        *view.imp().active_style.borrow_mut() = Some(active);
        view
    }

    pub fn update(&self, scene: Scene) {
        let resize = self.imp().scene.borrow().as_ref()
            .is_none_or(|old| old.width != scene.width || old.height != scene.height);
        let region = gtk::cairo::Region::create();
        for rect in scene.layout.input_region() {
            let _ = region.union_rectangle(&gtk::cairo::RectangleInt::new(rect.x, rect.y, rect.width, rect.height));
        }
        // Launch bounce also draws above the normal icon rectangle.
        for (icon, rect) in scene.icons.iter().zip(&scene.layout.icons) {
            if icon.bounce > 0.0 {
                let rect = motion::Rect { y: rect.y - icon.bounce, ..*rect }.outward();
                let _ = region.union_rectangle(&gtk::cairo::RectangleInt::new(rect.x, rect.y, rect.width, rect.height));
            }
        }
        if let Some(surface) = self.native().and_then(|native| native.surface()) {
            surface.set_input_region(&region);
        }
        *self.imp().scene.borrow_mut() = Some(scene);
        if resize { self.queue_resize(); }
        self.queue_draw();
    }
}
