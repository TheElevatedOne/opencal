//! The tray icon (StatusNotifierItem over D-Bus: Waybar, KDE, GNOME with the AppIndicator
//! extension, …). It runs on ksni's own thread and sends its clicks to the GTK main loop.

use gtk::glib;
use gtk::prelude::*;
use ksni::blocking::TrayMethods;

#[derive(Clone, Copy, Debug)]
pub enum TrayAction {
    Toggle,
    Show,
    NewEvent,
    NewTask,
    Sync,
    Quit,
}

struct Tray {
    tx: async_channel::Sender<TrayAction>,
    pixmap: Vec<ksni::Icon>,
    tooltip: String,
}

impl Tray {
    fn item(label: &str, icon: &str, action: TrayAction) -> ksni::MenuItem<Self> {
        ksni::menu::StandardItem {
            label: label.to_string(),
            icon_name: icon.to_string(),
            activate: Box::new(move |t: &mut Self| {
                let _ = t.tx.send_blocking(action);
            }),
            ..Default::default()
        }
        .into()
    }
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        crate::APP_ID.to_string()
    }

    fn title(&self) -> String {
        "opencal".to_string()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::ApplicationStatus
    }

    fn icon_name(&self) -> String {
        crate::APP_ID.to_string()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.pixmap.clone()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "opencal".into(),
            description: self.tooltip.clone(),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.tx.send_blocking(TrayAction::Toggle);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        vec![
            Self::item("Open opencal", "", TrayAction::Show),
            ksni::MenuItem::Separator,
            Self::item("New Event…", "appointment-new", TrayAction::NewEvent),
            Self::item("New Task…", "list-add", TrayAction::NewTask),
            Self::item("Sync Now", "view-refresh", TrayAction::Sync),
            ksni::MenuItem::Separator,
            Self::item("Quit opencal", "application-exit", TrayAction::Quit),
        ]
    }
}

pub struct TrayHandle(ksni::blocking::Handle<Tray>);

impl TrayHandle {
    /// Text shown when hovering the icon (e.g. the next event).
    pub fn set_tooltip(&self, text: String) {
        self.0.update(move |t| t.tooltip = text);
    }

    pub fn shutdown(self) {
        let _ = self.0.shutdown();
    }
}

/// Register the icon. `on_action` runs on the GTK main loop for each click. Fails when the
/// session has no StatusNotifierWatcher (no tray).
pub fn spawn(on_action: impl Fn(TrayAction) + 'static) -> Result<TrayHandle, String> {
    let (tx, rx) = async_channel::unbounded();
    let tray = Tray {
        tx,
        pixmap: pixmap(),
        tooltip: String::new(),
    };
    let handle = tray.spawn().map_err(|e| e.to_string())?;
    glib::spawn_future_local(async move {
        while let Ok(action) = rx.recv().await {
            on_action(action);
        }
    });
    Ok(TrayHandle(handle))
}

/// The icon as ARGB32 pixmaps, for trays that can't find it by name (not installed yet).
fn pixmap() -> Vec<ksni::Icon> {
    let bytes =
        glib::Bytes::from_static(include_bytes!("../../data/icons/org.opencal.OpenCal.svg"));
    let Ok(texture) = gtk::gdk::Texture::from_bytes(&bytes) else {
        return Vec::new();
    };
    let (width, height) = (texture.width(), texture.height());
    let mut downloader = gtk::gdk::TextureDownloader::new(&texture);
    downloader.set_format(gtk::gdk::MemoryFormat::R8g8b8a8);
    let (rgba, stride) = downloader.download_bytes();
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let i = y * stride + x * 4;
            data.extend_from_slice(&[rgba[i + 3], rgba[i], rgba[i + 1], rgba[i + 2]]);
        }
    }
    vec![ksni::Icon {
        width,
        height,
        data,
    }]
}
