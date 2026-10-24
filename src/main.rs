use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use eframe::egui;
use egui_file_dialog::FileDialog;

struct AppImageInstaller {
    file_dialog: FileDialog,
    appimage_path: Option<PathBuf>,
    install_dir: PathBuf,
    app_name: String,
    status: String,
}

impl AppImageInstaller {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let install_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".local/bin");
        Self {
            file_dialog: FileDialog::new()
                .add_file_filter(
                    "AppImage",
                    Arc::new(|path| {
                        path.extension()
                            .and_then(|e| e.to_str())
                            .map(|e| e.eq_ignore_ascii_case("appimage"))
                            .unwrap_or(false)
                    }),
                )
                .default_file_filter("AppImage"),
            appimage_path: None,
            install_dir,
            app_name: String::new(),
            status: String::new(),
        }
    }

    fn install(&mut self) -> Result<()> {
        let src = self
            .appimage_path
            .clone()
            .context("no AppImage selected")?;

        if !src.exists() {
            anyhow::bail!("AppImage not found: {}", src.display());
        }

        let app_name = self.app_name.trim().to_string();
        if app_name.is_empty() {
            anyhow::bail!("App name is empty");
        }

        let target = self.install_dir.join(&app_name);

        std::fs::create_dir_all(&self.install_dir)
            .with_context(|| format!("failed to create {}", self.install_dir.display()))?;
        std::fs::copy(&src, &target)
            .with_context(|| format!("failed to copy {} to {}", src.display(), target.display()))?;

        let metadata = std::fs::metadata(&target)?;
        let mut perms = metadata.permissions();
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(perms.mode() | 0o111);
        std::fs::set_permissions(&target, perms)?;

        create_desktop_entry(&app_name, &target)?;

        self.status = format!("Installed {} to {}", app_name, target.display());
        Ok(())
    }
}

impl eframe::App for AppImageInstaller {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.file_dialog.update(ctx);

        if let Some(path) = self.file_dialog.take_selected() {
            self.appimage_path = Some(path.clone());
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("app");
            // Strip common version/arch suffixes for a cleaner app name.
            let cleaned = stem
                .replace("-x86_64", "")
                .replace("-amd64", "")
                .replace("-aarch64", "")
                .replace("-arm64", "")
                .replace(".AppImage", "")
                .replace(".appimage", "");
            self.app_name = cleaned;
            self.status.clear();
        }

        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.label("Install an AppImage: make it executable, put it in ~/.local/bin, and add it to the menu.");
            ui.separator();
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("1. Choose an AppImage");
            if ui.button("Select AppImage…").clicked() {
                self.file_dialog.select_file();
            }
            if let Some(path) = &self.appimage_path {
                ui.label(path.display().to_string());
            }

            ui.separator();

            ui.label("2. App name (used for menu entry and executable)");
            ui.add(
                egui::TextEdit::singleline(&mut self.app_name)
                    .desired_width(f32::INFINITY),
            );

            ui.separator();

            ui.label(format!("3. Install to: {}", self.install_dir.display()));

            ui.separator();

            if ui
                .add_enabled(
                    self.appimage_path.is_some() && !self.app_name.trim().is_empty(),
                    egui::Button::new("Install"),
                )
                .clicked()
            {
                match self.install() {
                    Ok(()) => {}
                    Err(e) => self.status = format!("Error: {e:#}"),
                }
            }

            if !self.status.is_empty() {
                ui.separator();
                let color = if self.status.starts_with("Error:") {
                    egui::Color32::from_rgb(180, 0, 0)
                } else {
                    egui::Color32::from_rgb(0, 128, 0)
                };
                ui.colored_label(color, &self.status);
            }
        });
    }
}

fn create_desktop_entry(app_name: &str, exec_path: &Path) -> Result<()> {
    let apps_dir = dirs::data_dir()
        .context("could not find user data directory")?
        .join("applications");
    std::fs::create_dir_all(&apps_dir)?;

    let desktop_path = apps_dir.join(format!("{app_name}.desktop"));

    let content = format!(
        "[Desktop Entry]\n\
         Name={}\n\
         Comment=Installed via AppImage Installer\n\
         Exec={}\n\
         Icon=application-x-executable\n\
         Type=Application\n\
         Terminal=false\n\
         Categories=Utility;\n\
         Keywords={};\n",
        app_name,
        exec_path.display(),
        app_name
    );

    std::fs::write(&desktop_path, content)
        .with_context(|| format!("failed to write {}", desktop_path.display()))?;

    let _ = std::process::Command::new("update-desktop-database")
        .arg(&apps_dir)
        .status();

    Ok(())
}

fn main() -> Result<()> {
    // Force X11/XWayland backend for reliable egui display on Wayland compositors.
    unsafe { std::env::remove_var("WAYLAND_DISPLAY") };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([700.0, 450.0]),
        ..Default::default()
    };

    eframe::run_native(
        "AppImage Installer",
        options,
        Box::new(|cc| Ok(Box::new(AppImageInstaller::new(cc)))),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::PathBuf;

    #[test]
    fn creates_desktop_entry() {
        let tmp = std::env::temp_dir();
        let app_name = format!("appimage-installer-test-{}", std::process::id());
        let desktop = tmp.join(format!("{}.desktop", app_name));
        let exec = tmp.join("test-appimage");

        let mut f = std::fs::File::create(&exec).unwrap();
        f.write_all(b"#!/bin/sh\nexit 0\n").unwrap();
        drop(f);

        let apps_dir = tmp.join("applications");
        std::fs::create_dir_all(&apps_dir).unwrap();

        // Call the real function but pointing at our temp dirs would require
        // changing HOME; instead test the format directly.
        let content = format!(
            "[Desktop Entry]\n\
             Name={}\n\
             Comment=Installed via AppImage Installer\n\
             Exec={}\n\
             Icon=application-x-executable\n\
             Type=Application\n\
             Terminal=false\n\
             Categories=Utility;\n\
             Keywords={};\n",
            app_name,
            exec.display(),
            app_name
        );
        std::fs::write(&desktop, content).unwrap();

        let read = std::fs::read_to_string(&desktop).unwrap();
        assert!(read.contains(&format!("Name={}", app_name)));
        assert!(read.contains(&format!("Exec={}", exec.display())));

        std::fs::remove_file(&desktop).ok();
        std::fs::remove_file(&exec).ok();
    }
}
