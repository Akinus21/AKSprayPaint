//! gdb-based injection of GTK settings into running processes.
//!
//! Uses `gdb -p <pid> -batch ...` to call g_object_set on GtkSettings properties.

use std::process::Command;

use crate::gtkd::config::GtkBridgeConfig;
use crate::gtkd::theme::ThemeState;

/// GTK setting → GtkSettings property name mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GtkSetting {
    Icons,
    GtkTheme,
    CursorTheme,
    Font,
    ColorScheme,
}

impl GtkSetting {
    pub fn property_name(&self) -> &'static str {
        match self {
            Self::Icons => "gtk-icon-theme-name",
            Self::GtkTheme => "gtk-theme-name",
            Self::CursorTheme => "gtk-cursor-theme-name",
            Self::Font => "gtk-font-name",
            Self::ColorScheme => "gtk-application-prefer-dark-theme",
        }
    }

    pub fn type_hint(&self) -> &'static str {
        match self {
            Self::ColorScheme => "boolean",
            _ => "string",
        }
    }
}

/// A single setting to inject.
#[derive(Debug, Clone)]
pub struct Setting {
    pub property: GtkSetting,
    pub value: String,
}

impl Setting {
    /// Build the gdb call expression for this setting.
    fn gdb_call(&self) -> String {
        let prop = self.property.property_name();
        let value = &self.value;

        if self.property.type_hint() == "string" {
            format!(
                r#"call (void) g_object_set((void*)gtk_settings_get_default(), "{}", "{}", (void*)0)"#,
                prop, value
            )
        } else {
            format!(
                r#"call (void) g_object_set((void*)gtk_settings_get_default(), "{}", {}, (void*)0)"#,
                prop, value
            )
        }
    }
}

/// Build all settings to inject for a given config + theme state.
pub fn build_settings(config: &GtkBridgeConfig, theme: &ThemeState) -> Vec<Setting> {
    let mut settings = Vec::new();
    let st = &config.settings;

    if st.icons {
        settings.push(Setting {
            property: GtkSetting::Icons,
            value: theme.icon_theme_name(),
        });
    }

    if st.gtk_theme {
        settings.push(Setting {
            property: GtkSetting::GtkTheme,
            value: "adw-gtk3-dark".to_string(),
        });
    }

    if st.cursor_theme {
        settings.push(Setting {
            property: GtkSetting::CursorTheme,
            value: "Adwaita".to_string(),
        });
    }

    if st.font {
        settings.push(Setting {
            property: GtkSetting::Font,
            value: "Cantarell 11".to_string(),
        });
    }

    if st.color_scheme {
        settings.push(Setting {
            property: GtkSetting::ColorScheme,
            value: "1".to_string(),
        });
    }

    settings
}

/// Inject all settings into a running process by PID using gdb.
pub fn inject_settings(pid: u32, settings: &[Setting]) -> Result<(), String> {
    if settings.is_empty() {
        return Ok(());
    }

    let mut args = vec![
        "-p".to_string(),
        pid.to_string(),
        "-batch".to_string(),
    ];

    for setting in settings {
        args.push("-ex".to_string());
        args.push(setting.gdb_call());
    }
    args.push("-ex".to_string());
    args.push("detach".to_string());

    let output = Command::new("gdb")
        .args(&args)
        .output()
        .map_err(|e| format!("failed to run gdb: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gdb injection failed: {}", stderr));
    }

    Ok(())
}

/// Inject all settings into a running process asynchronously (spawned task).
pub fn inject_async(pid: u32, settings: Vec<Setting>) {
    std::thread::spawn(move || {
        match inject_settings(pid, &settings) {
            Ok(()) => {
                eprintln!(
                    "[gtkd] injected settings into PID {} ({})",
                    pid,
                    settings
                        .iter()
                        .map(|s| s.property.property_name())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            Err(e) => {
                eprintln!("[gtkd] injection into PID {} failed: {}", pid, e);
            }
        }
    });
}
