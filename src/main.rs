use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use eframe::egui;
use egui_file_dialog::FileDialog;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SourceKind {
    AppImage,
    TarGz,
    Unknown,
}

struct AppImageInstaller {
    file_dialog: FileDialog,
    source_path: Option<PathBuf>,
    source_kind: SourceKind,
    app_name: String,
    detected_executables: Vec<PathBuf>,
    selected_executable: Option<usize>,
    status: String,
}

impl AppImageInstaller {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
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
                .add_file_filter(
                    "Tar/Gz archive",
                    Arc::new(|path| {
                        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                        name.to_lowercase().ends_with(".tar.gz")
                    }),
                )
                .default_file_filter("AppImage"),
            source_path: None,
            source_kind: SourceKind::Unknown,
            app_name: String::new(),
            detected_executables: Vec::new(),
            selected_executable: None,
            status: String::new(),
        }
    }

    fn set_source(&mut self, path: PathBuf) {
        self.source_kind = detect_kind(&path);
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("app")
            .to_string();
        // Strip .tar from stem if present.
        let stem = stem.trim_end_matches(".tar").to_string();
        let cleaned = stem
            .replace("-x86_64", "")
            .replace("-amd64", "")
            .replace("-aarch64", "")
            .replace("-arm64", "")
            .replace(".AppImage", "")
            .replace(".appimage", "");
        self.app_name = cleaned;
        self.source_path = Some(path);
        self.detected_executables.clear();
        self.selected_executable = None;
        self.status.clear();
    }

    fn install(&mut self) -> Result<()> {
        let src = self.source_path.clone().context("no file selected")?;
        let app_name = self.app_name.trim().to_string();
        if app_name.is_empty() {
            anyhow::bail!("App name is empty");
        }

        match self.source_kind {
            SourceKind::AppImage => install_appimage(&app_name, &src),
            SourceKind::TarGz => {
                let idx = self.selected_executable.context("no executable selected")?;
                let exec = self
                    .detected_executables
                    .get(idx)
                    .context("invalid executable selection")?
                    .clone();
                install_targz(&app_name, &src, &exec)
            }
            SourceKind::Unknown => anyhow::bail!("unsupported file type"),
        }
    }

    fn scan_tar(&mut self) {
        self.detected_executables.clear();
        self.selected_executable = None;
        if let Some(path) = &self.source_path {
            if self.source_kind == SourceKind::TarGz {
                match find_executables_in_targz(path) {
                    Ok(list) => {
                        self.detected_executables = list;
                        if self.detected_executables.len() == 1 {
                            self.selected_executable = Some(0);
                        }
                    }
                    Err(e) => self.status = format!("Error scanning archive: {e:#}"),
                }
            }
        }
    }
}

impl eframe::App for AppImageInstaller {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.file_dialog.update(ctx);

        if let Some(path) = self.file_dialog.take_selected() {
            self.set_source(path);
            self.scan_tar();
        }

        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.label("Install an AppImage or a .tar.gz archive to the applications menu.");
            ui.separator();
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("1. Choose a file");
            if ui.button("Select file…").clicked() {
                self.file_dialog.select_file();
            }
            if let Some(path) = &self.source_path {
                ui.label(format!(
                    "{} ({:?})",
                    path.display(),
                    self.source_kind
                ));
            }

            ui.separator();

            ui.label("2. App name");
            ui.add(
                egui::TextEdit::singleline(&mut self.app_name)
                    .desired_width(f32::INFINITY),
            );

            ui.separator();

            if self.source_kind == SourceKind::TarGz {
                ui.label("3. Select the main executable inside the archive");
                if self.detected_executables.is_empty() {
                    ui.label("(no executables found — the archive may be unusual)");
                } else {
                    egui::ScrollArea::vertical()
                        .max_height(200.0)
                        .show(ui, |ui| {
                            for (idx, exec) in self.detected_executables.iter().enumerate() {
                                let selected = self.selected_executable == Some(idx);
                                let label = exec.display().to_string();
                                if ui.selectable_label(selected, label).clicked() {
                                    self.selected_executable = Some(idx);
                                }
                            }
                        });
                }
                ui.separator();
            }

            let can_install = !self.app_name.trim().is_empty()
                && self.source_path.is_some()
                && match self.source_kind {
                    SourceKind::AppImage => true,
                    SourceKind::TarGz => self.selected_executable.is_some(),
                    SourceKind::Unknown => false,
                };

            if ui
                .add_enabled(can_install, egui::Button::new("Install"))
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

fn detect_kind(path: &Path) -> SourceKind {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();
    if name.ends_with(".appimage") {
        SourceKind::AppImage
    } else if name.ends_with(".tar.gz") {
        SourceKind::TarGz
    } else {
        SourceKind::Unknown
    }
}

fn install_appimage(app_name: &str, src: &Path) -> Result<()> {
    let bin_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".local/bin");
    let target = bin_dir.join(app_name);

    std::fs::create_dir_all(&bin_dir)
        .with_context(|| format!("failed to create {}", bin_dir.display()))?;
    std::fs::copy(src, &target)
        .with_context(|| format!("failed to copy {} to {}", src.display(), target.display()))?;

    make_executable(&target)?;
    create_desktop_entry(app_name, &target)?;

    Ok(())
}

fn install_targz(app_name: &str, src: &Path, exec: &Path) -> Result<()> {
    let lib_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(format!(".local/lib/{app_name}"));

    // Clean and re-extract.
    let _ = std::fs::remove_dir_all(&lib_dir);
    std::fs::create_dir_all(&lib_dir)?;
    extract_targz(src, &lib_dir)
        .with_context(|| format!("failed to extract {}", src.display()))?;

    let exec_path = lib_dir.join(exec);
    if !exec_path.exists() {
        anyhow::bail!("selected executable not found after extraction");
    }

    create_desktop_entry(app_name, &exec_path)?;

    Ok(())
}

fn extract_targz(src: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::open(src)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);
    archive.unpack(dest)?;
    Ok(())
}

fn find_executables_in_targz(src: &Path) -> Result<Vec<PathBuf>> {
    let file = std::fs::File::open(src)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);

    let mut executables = Vec::new();
    for entry in archive.entries()? {
        let entry = entry?;
        let header = entry.header();
        let mode = header.mode().unwrap_or(0);
        let is_exec = mode & 0o111 != 0;
        if !is_exec {
            continue;
        }
        let path = entry.path()?;
        if path.file_name().is_none() {
            continue;
        }
        // Skip libraries and build artifacts.
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if name.ends_with(".so") || name.contains(".so.") {
            continue;
        }
        executables.push(path.into_owned());
    }

    // Sort so shorter/more-likely-main executables appear first.
    executables.sort_by(|a, b| {
        let a_depth = a.components().count();
        let b_depth = b.components().count();
        a_depth.cmp(&b_depth).then_with(|| a.as_os_str().len().cmp(&b.as_os_str().len()))
    });

    Ok(executables)
}

fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = std::fs::metadata(path)?;
    let mut perms = metadata.permissions();
    perms.set_mode(perms.mode() | 0o111);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

fn create_desktop_entry(app_name: &str, exec_path: &Path) -> Result<()> {
    let apps_dir = dirs::data_dir()
        .context("could not find user data directory")?
        .join("applications");
    std::fs::create_dir_all(&apps_dir)?;

    let desktop_path = apps_dir.join(format!("{app_name}.desktop"));

    // Quote the Exec path so spaces are handled correctly.
    let exec_quoted = format!("\"{}\"", exec_path.display());

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
        exec_quoted,
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
    unsafe { std::env::remove_var("WAYLAND_DISPLAY") };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([700.0, 500.0]),
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
    use std::path::PathBuf;

    #[test]
    fn desktop_entry_quotes_exec_path() {
        let app_name = "Test App";
        let exec = PathBuf::from("/home/user/.local/bin/Test App");
        let content = format!(
            "[Desktop Entry]\n\
             Name={}\n\
             Comment=Installed via AppImage Installer\n\
             Exec=\"{}\"\n",
            app_name,
            exec.display()
        );
        assert!(content.contains("Exec=\"/home/user/.local/bin/Test App\""));
    }
}
