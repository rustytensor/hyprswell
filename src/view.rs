use gtk::prelude::*;
use gtk::subclass::prelude::*;
use hyprswell::{grid, motion, rules};
use std::cell::RefCell;

#[derive(Clone)]
pub struct Icon {
    pub paintable: Option<gtk::gdk::Paintable>,
    pub grid: Option<grid::Look>,
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
    pub shelf: Option<motion::Rect>,
}

mod imp {
    use super::*;
    #[derive(Default)]
    pub struct DockView {
        pub scene: RefCell<Option<Scene>>,
        pub bar_style: RefCell<Option<gtk::Box>>,
        pub indicator_style: RefCell<Option<gtk::Label>>,
        pub active_style: RefCell<Option<gtk::Box>>,
        pub shelf_style: RefCell<Option<gtk::Box>>,
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
        fn size_allocate(&self, _: i32, _: i32, _: i32) {
            let scene = self.scene.borrow();
            let Some(scene) = scene.as_ref() else { return; };
            allocate_style(self.bar_style.borrow().as_ref().unwrap(), scene.layout.bar);
            let shelf = self.shelf_style.borrow();
            let shelf = shelf.as_ref().unwrap();
            shelf.set_child_visible(scene.shelf.is_some());
            if let Some(rect) = scene.shelf { allocate_style(shelf, rect); }
            let active = self.active_style.borrow();
            let active = active.as_ref().unwrap();
            let icon = scene.icons.iter().zip(&scene.layout.icons).find(|(icon, _)| icon.active);
            active.set_child_visible(icon.is_some());
            if let Some((_, rect)) = icon {
                allocate_style(active, motion::Rect {
                    x: rect.x + rect.width / 2.0 - 10.0, y: scene.indicator_y + 4.0,
                    width: 20.0, height: 3.0,
                });
            }
            let mut child = self.obj().first_child();
            while let Some(widget) = child {
                child = widget.next_sibling();
                if let Some(menu) = widget.downcast_ref::<gtk::Popover>() { menu.present(); }
            }
        }
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let scene = self.scene.borrow();
            let Some(scene) = scene.as_ref() else { return; };
            let styles = self.bar_style.borrow();
            let bar = styles.as_ref().unwrap();
            let accent = self.indicator_style.borrow().as_ref().unwrap().color();
            self.obj().snapshot_child(bar, snapshot);
            if let Some(shelf) = scene.shelf {
                for (icon, rect) in scene.icons.iter().zip(&scene.layout.icons) {
                    let actual = motion::Rect { y: rect.y - icon.bounce, ..*rect };
                    let reflected = if icon.grid.is_some() { grid::reflection(actual) }
                        else { motion::reflection(actual, shelf.y) };
                    let clip = gtk::graphene::Rect::new(reflected.visible.x as f32, reflected.visible.y as f32,
                        reflected.visible.width as f32, reflected.visible.height as f32);
                    let stops = reflected.stops.map(|(offset, alpha)| gtk::gsk::ColorStop::new(
                        offset as f32, gtk::gdk::RGBA::new(1.0, 1.0, 1.0, alpha as f32)));
                    snapshot.push_clip(&clip);
                    // GTK records the mask before the first pop, then its source before the second.
                    snapshot.push_mask(gtk::gsk::MaskMode::Alpha);
                    snapshot.append_linear_gradient(&clip,
                        &gtk::graphene::Point::new(reflected.visible.x as f32, reflected.visible.y as f32),
                        &gtk::graphene::Point::new(reflected.visible.x as f32,
                            (reflected.visible.y + reflected.visible.height) as f32), &stops);
                    snapshot.pop();
                    snapshot.save();
                    snapshot.translate(&gtk::graphene::Point::new(reflected.image.x as f32, reflected.flip_y as f32));
                    snapshot.scale(1.0, -1.0);
                    paint_icon(icon, snapshot, reflected.image.width, reflected.image.height, bar.color(), accent);
                    snapshot.restore();
                    snapshot.pop();
                    snapshot.pop();
                }
                self.obj().snapshot_child(self.shelf_style.borrow().as_ref().unwrap(), snapshot);
            }
            for (icon, rect) in scene.icons.iter().zip(&scene.layout.icons) {
                snapshot.save();
                if icon.shadow {
                    snapshot.push_shadow(&[gtk::gsk::Shadow::new(
                        gtk::gdk::RGBA::new(0.0, 0.0, 0.0, 0.35), 0.0, 6.0, 10.0)]);
                }
                snapshot.translate(&gtk::graphene::Point::new(rect.x as f32, (rect.y - icon.bounce) as f32));
                paint_icon(icon, snapshot, rect.width, rect.height, bar.color(), accent);
                if icon.shadow { snapshot.pop(); }
                snapshot.restore();
            }
            let color = accent;
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
            }
            self.obj().snapshot_child(self.active_style.borrow().as_ref().unwrap(), snapshot);
        }
    }
}

fn paint_icon(icon: &Icon, snapshot: &gtk::Snapshot, width: f64, height: f64,
              color: gtk::gdk::RGBA, accent: gtk::gdk::RGBA) {
    if let Some(look) = icon.grid {
        let (dots, radius) = grid::dots(motion::Rect { x: 0.0, y: 0.0, width, height }, look);
        let cr = snapshot.append_cairo(&gtk::graphene::Rect::new(0.0, 0.0, width as f32, height as f32));
        let mix = |a: f32, b: f32| a as f64 + (b - a) as f64 * look.accent;
        cr.set_source_rgba(mix(color.red(), accent.red()), mix(color.green(), accent.green()),
                           mix(color.blue(), accent.blue()), mix(color.alpha(), accent.alpha()));
        for (x, y) in dots {
            cr.new_sub_path();
            cr.arc(x, y, radius, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        }
    } else if let Some(paintable) = &icon.paintable {
        if let Some(symbolic) = paintable.dynamic_cast_ref::<gtk::SymbolicPaintable>() {
            symbolic.snapshot_symbolic(snapshot, width, height, &[color; 4]);
        } else { paintable.snapshot(snapshot, width, height); }
    }
}

// GTK 4's widget snapshot paints CSS backgrounds and borders without style contexts.
fn allocate_style(widget: &gtk::Box, rect: motion::Rect) {
    let width = rect.width.ceil().max(1.0) as i32;
    let height = rect.height.ceil().max(1.0) as i32;
    let transform = gtk::gsk::Transform::new()
        .translate(&gtk::graphene::Point::new(rect.x as f32, rect.y as f32))
        .scale(rect.width as f32 / width as f32, rect.height as f32 / height as f32);
    widget.allocate(width, height, -1, Some(transform));
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
        bar.set_can_target(false);
        bar.set_parent(&view);
        let indicator = gtk::Label::new(None);
        indicator.add_css_class("dock-indicator");
        *view.imp().bar_style.borrow_mut() = Some(bar);
        let shelf = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        shelf.add_css_class("dock-shelf");
        shelf.set_can_target(false);
        shelf.set_child_visible(false);
        shelf.set_parent(&view);
        *view.imp().shelf_style.borrow_mut() = Some(shelf);
        *view.imp().indicator_style.borrow_mut() = Some(indicator);
        let active = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        active.add_css_class("dock-item");
        active.add_css_class("active");
        active.set_can_target(false);
        active.set_child_visible(false);
        active.set_parent(&view);
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
        if resize { self.queue_resize(); } else { self.queue_allocate(); }
        self.queue_draw();
    }
}
