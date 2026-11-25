//! Validate and publish the planned Compose configuration, then activate it.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

use super::{
    ENV_MARKER, EnvAction, EnvState, IMAGE, INSTALLED_TURN, NONE, OTHER_KEYS, Run, Step, display,
};
use crate::cli::{KEYS, Key};
use crate::envfile;
use crate::install_plan::{self, InstallPlan};
use crate::sys;
use crate::ui::{Fd, Stop};

impl Run {
    fn prepare_directory(&mut self) -> Step {
        let dir = display(&self.install_dir);
        if self.env_action == EnvAction::New && !self.owns_container {
            self.ui.section(&format!("Installing into {dir}"));
        } else if self.updating {
            self.ui.section(&format!("Updating {dir}"));
        } else {
            self.ui.section(&format!("Using {dir}"));
        }
        if fs::create_dir_all(&self.install_dir).is_err() {
            return Err(self.die(
                format!("Could not create {dir}"),
                &["Choose another directory with --dir."],
            ));
        }
        if std::env::set_current_dir(&self.install_dir).is_err() {
            return Err(self.die(format!("Could not enter {dir}"), NONE));
        }
        if let Ok(canonical) = fs::canonicalize(".") {
            self.install_dir = canonical;
        }
        let dir = display(&self.install_dir);
        // SAFETY: access only reads the C string passed.
        let writable = std::ffi::CString::new(".")
            .is_ok_and(|p| unsafe { libc::access(p.as_ptr(), libc::W_OK) } == 0);
        if !writable {
            return Err(self.die(
                format!("This user cannot write to {dir}"),
                &["Run the installer as a user who can, or choose another directory with --dir."],
            ));
        }
        Ok(())
    }

    pub(super) fn deploy(&mut self) -> Step {
        self.prepare_directory()?;
        let install_dir = self.install_dir.clone();
        let result = self.deploy_inner(&install_dir);
        match InstallPlan::recover(&install_dir) {
            Ok(true) => self.ui.ok("Restored the previous installation files", NONE),
            Ok(false) => {}
            Err(error) => {
                return Err(self.die(
                    "Could not clean up or restore installation files",
                    &[error.to_string()],
                ));
            }
        }
        result
    }

    fn deploy_inner(&mut self, install_dir: &Path) -> Step {
        let mut plan = InstallPlan::new(install_dir).map_err(|error| {
            self.die(
                "Could not stage the installation files",
                &[error.to_string()],
            )
        })?;
        self.stage_files(&mut plan).map_err(|error| {
            self.die(
                "Could not stage the installation files",
                &[error.to_string()],
            )
        })?;
        self.clear_compose_environment(&plan.path);
        let view = plan.view();
        let mut staged = self.validate_plan(&view)?;
        let from = view.to_string_lossy().replace('$', "$$");
        let to = install_dir.to_string_lossy().replace('$', "$$");
        rebase_model_paths(&mut staged, Path::new(&from), Path::new(&to));
        if self.o.start {
            self.prepare_images(&plan, &view, &staged)?;
        }
        let backups = plan.publish(&self.run_stamp).map_err(|error| {
            self.die(
                "Could not publish the installation files",
                &[error.to_string()],
            )
        })?;
        // Check again at the final paths before committing the publication.
        // This catches a relative include or env_file that resolved differently.
        let mut live = self.validate_plan(install_dir)?;
        if self.o.build {
            for model in [&mut staged, &mut live] {
                if let Some(service) = model["services"]["stellarr"].as_object_mut() {
                    service.remove("build");
                    service.remove("pull_policy");
                }
            }
        }
        if staged != live {
            return Err(self.die(
                "Docker Compose resolves the installation differently at its final path",
                &["Check relative paths in your Compose override."],
            ));
        }
        plan.commit().map_err(|error| {
            self.die(
                "Could not finalize the installation files",
                &[error.to_string()],
            )
        })?;
        for (name, backup) in backups {
            self.ui.ok(&format!("Previous {name} kept as"), &[backup]);
        }
        self.show_ports();
        if self.o.start {
            self.activate()?;
        }
        Ok(())
    }

    fn prepare_images(
        &mut self,
        plan: &InstallPlan,
        directory: &Path,
        model: &serde_json::Value,
    ) -> Step {
        let services = model["services"]
            .as_object()
            .ok_or_else(|| self.die("Docker Compose has no active Stellarr services", NONE))?;
        let mut builds = vec!["build"];
        let mut pulls = vec!["pull"];
        let mut images = Vec::new();
        let mut build_paths = serde_json::Map::new();
        let image_prefix = format!("{IMAGE}:");
        for (name, service) in services {
            if name == "stellarr" && !self.o.build {
                continue;
            }
            let build = service["build"].as_object();
            let image = match (service["image"].as_str(), build) {
                (Some(image), _) => image.to_owned(),
                (None, Some(_)) => format!("stellarr-{name}"),
                (None, None) => continue,
            };
            if let Some(build) = build {
                let targets = std::iter::once(image.as_str()).chain(
                    service["build"]["tags"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(serde_json::Value::as_str),
                );
                if targets.into_iter().any(|tag| {
                    tag == IMAGE
                        || tag.starts_with(&image_prefix)
                        || (name != "stellarr" && tag == self.target_image)
                }) {
                    return Err(self.die(
                        format!("Building {name} would replace a Stellarr image"),
                        NONE,
                    ));
                }
                let paths = ["context", "additional_contexts", "dockerfile"]
                    .into_iter()
                    .filter_map(|key| build.get(key).map(|value| (key.to_owned(), value.clone())))
                    .collect::<serde_json::Map<_, _>>();
                build_paths.insert(name.clone(), serde_json::json!({"build": paths}));
                builds.push(name);
            } else if service["pull_policy"].as_str() == Some("never") {
                self.require_service_image(name, &image)?;
                continue;
            } else {
                pulls.push(name);
            }
            images.push((name, image));
        }
        if !self.o.build {
            self.pull_image(directory)?;
        }
        if pulls.len() > 1 {
            let (ok, log) = sys::logged(self.compose_at(directory, &pulls));
            if !ok {
                self.ui.fail(
                    "Could not pull images for additional Compose services",
                    NONE,
                );
                self.ui.block(Fd::Err, &log);
                return Err(Stop(1));
            }
        }
        if builds.len() > 1 {
            plan.write_build_paths(build_paths).map_err(|error| {
                self.die(
                    "Could not prepare the Compose build contexts",
                    &[error.to_string()],
                )
            })?;
            self.ui.text(
                Fd::Out,
                &["Building Compose service images. Docker's output follows."],
            );
            if !sys::shown(self.compose_at(directory, &builds)) {
                return Err(self.die("Building the image failed", &["Docker's output is above."]));
            }
        }
        for (service, image) in images {
            self.require_service_image(service, &image)?;
        }
        Ok(())
    }

    fn require_service_image(&mut self, service: &str, image: &str) -> Step {
        if sys::quiet(self.docker(["image", "inspect", image])) {
            Ok(())
        } else {
            Err(self.die(
                format!("Compose service {service} requires local image {image}"),
                NONE,
            ))
        }
    }

    fn stage_files(&mut self, plan: &mut InstallPlan) -> std::io::Result<()> {
        let managed = !self.dir_is_checkout;
        let base = self
            .staged_compose
            .take()
            .map_or_else(|| fs::read(self.install_dir.join("docker-compose.yml")), Ok)?;
        plan.candidate("docker-compose.yml", &base, 0o644, managed)?;
        if self.two_port_layout && self.turn_published && managed {
            let overlay = self
                .staged_turn
                .take()
                .map_or_else(|| fs::read(self.install_dir.join(INSTALLED_TURN)), Ok)?;
            plan.candidate(INSTALLED_TURN, &overlay, 0o644, true)?;
        }
        let example = match self.staged_example.take() {
            Some(example) => Some(example),
            None => install_plan::read(&self.install_dir.join(".env.example"))?,
        };
        if let Some(example) = &example {
            plan.candidate(".env.example", example, 0o644, managed)?;
        }
        let mut env = match self.env_action {
            EnvAction::New | EnvAction::Replace => self.new_env(example.as_deref()),
            EnvAction::Keep => self.existing_env.clone(),
            EnvAction::Adopt => envfile::with_marker(&self.existing_env, ENV_MARKER),
        };
        if self.keeps_env() {
            let changes = self.env_changes();
            if !changes.is_empty() {
                env = envfile::with_values(&env, &changes);
            }
        }
        let current = install_plan::read(&self.install_dir.join(".env"))?;
        let expected = (self.env_state != EnvState::None).then_some(self.existing_env.as_slice());
        if current.as_deref() != expected {
            return Err(std::io::Error::other(".env changed during installation"));
        }
        plan.candidate(".env", &env, 0o600, true)?;
        if self.o.build {
            plan.set_build_context()?;
        }
        plan.mirror()
    }

    fn validate_plan(&mut self, path: &Path) -> Step<serde_json::Value> {
        let output = sys::capture(self.compose_at(path, &["config", "--format", "json"])).map_err(
            |error| self.die("Could not run Docker Compose config", &[error.to_string()]),
        )?;
        if !output.status.success() {
            let dir = display(&self.install_dir);
            self.ui.fail(
                &format!("Docker Compose cannot read the configuration in {dir}"),
                NONE,
            );
            self.ui.block(Fd::Err, &output.stderr);
            return Err(Stop(1));
        }
        let model: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|error| {
            self.die(
                "Could not read Docker Compose's effective configuration",
                &[error.to_string()],
            )
        })?;
        let service = &model["services"]["stellarr"];
        let correct_mount = service["volumes"].as_array().is_some_and(|volumes| {
            let mut data = volumes
                .iter()
                .filter(|volume| volume["target"].as_str() == Some("/data/stellarr"));
            data.next().is_some_and(|volume| {
                volume["type"].as_str() == Some("volume")
                    && volume["source"].as_str() == Some("stellarr_data")
                    && volume["read_only"].as_bool() != Some(true)
                    && volume["volume"]["subpath"].is_null()
            }) && data.next().is_none()
        });
        let volume = &model["volumes"]["stellarr_data"];
        if model["name"].as_str() != Some("stellarr")
            || volume["name"].as_str() != Some("stellarr_stellarr_data")
            || volume["driver"]
                .as_str()
                .is_some_and(|driver| driver != "local")
            || volume["driver_opts"]
                .as_object()
                .is_some_and(|options| !options.is_empty())
            || service["container_name"].as_str() != Some("stellarr")
            || !correct_mount
        {
            return Err(self.die(
                "Docker Compose changes the Stellarr project, container, or data volume",
                NONE,
            ));
        }
        if path != self.install_dir
            && self.o.build
            && service["build"]["context"].as_str()
                != Some(&self.install_dir.to_string_lossy().replace('$', "$$"))
        {
            return Err(self.die(
                "Docker Compose must build Stellarr from this checkout",
                NONE,
            ));
        }
        if service["image"].as_str() != Some(self.target_image.as_str()) {
            return Err(self.die(
                format!("Docker Compose should use image {}", self.target_image),
                &[format!("Effective image: {}", service["image"])],
            ));
        }
        Ok(model)
    }

    /// The changes an existing .env needs: what this run was given, and the
    /// release on an update. A value it already has is no change.
    fn env_changes(&self) -> Vec<(String, String)> {
        let mut changes = Vec::new();
        for key in KEYS {
            if self.given(key) && (!self.has(key.name()) || self.exist(key.name()) != self.set(key))
            {
                changes.push((key.name().to_string(), self.set(key).to_string()));
            }
        }
        if self.set_compose_file != self.exist("COMPOSE_FILE")
            || (self.set_compose_file.is_empty() && self.has("COMPOSE_FILE"))
        {
            changes.push(("COMPOSE_FILE".to_string(), self.set_compose_file.clone()));
        }
        if !self.has("STELLARR_VERSION") || self.exist("STELLARR_VERSION") != self.target_tag {
            changes.push(("STELLARR_VERSION".to_string(), self.target_tag.clone()));
        }
        for (key, value) in [
            ("STELLARR_TRACK", self.target_track.as_str()),
            (
                "STELLARR_IMAGE",
                if self.o.build {
                    self.target_image.as_str()
                } else {
                    ""
                },
            ),
        ] {
            if self.exist(key) != value || (value.is_empty() && self.has(key)) {
                changes.push((key.to_string(), value.to_string()));
            }
        }
        changes
    }

    fn new_env(&self, example: Option<&[u8]>) -> Vec<u8> {
        let mut out = format!("{ENV_MARKER}\n");
        for key in [Key::Domain, Key::PublicIp, Key::Language] {
            out.push_str(&format!("{}={}\n", key.name(), self.set(key)));
        }
        out.push_str(&format!("STELLARR_VERSION={}\n", self.target_tag));
        if !self.target_track.is_empty() {
            out.push_str(&format!("STELLARR_TRACK={}\n", self.target_track));
        }
        if self.o.build {
            out.push_str(&format!("STELLARR_IMAGE={}\n", self.target_image));
        }
        if !self.set_compose_file.is_empty() {
            out.push_str(&format!("COMPOSE_FILE={}\n", self.set_compose_file));
        }
        for key in [
            Key::HttpsPort,
            Key::PublicPort,
            Key::HttpPort,
            Key::MediaPort,
            Key::TurnEnabled,
            Key::TurnPort,
            Key::TurnsPort,
        ] {
            if !self.set(key).is_empty() {
                out.push_str(&format!("{}={}\n", key.name(), self.set(key)));
            }
        }
        let mut content = out.into_bytes();
        if let Some(example) = example {
            content.extend_from_slice(b"\n# Below is .env.example of the same version as docker-compose.yml, for reference.\n");
            let ours: Vec<&str> = KEYS
                .iter()
                .map(|k| k.name())
                .chain(OTHER_KEYS.iter().copied())
                .collect();
            content.extend_from_slice(&envfile::commented(example, &ours));
        }
        content
    }

    /// Compose takes a variable from the environment over the one in .env,
    /// and COMPOSE_PROJECT_NAME over the project's own name, which would
    /// start the installation on another, empty volume. The installer runs
    /// Compose on .env alone, as a later "docker compose up" from another
    /// shell would; what it was given in the environment is in .env by now.
    fn clear_compose_environment(&mut self, path: &Path) {
        let mut files = vec!["docker-compose.yml"];
        if self.two_port_layout && self.turn_published {
            files.push(self.turn_file());
        }
        self.removed_env = [
            "COMPOSE_PROJECT_NAME",
            "COMPOSE_FILE",
            "COMPOSE_PROFILES",
            "COMPOSE_ENV_FILES",
            "COMPOSE_DISABLE_ENV_FILE",
            "COMPOSE_PATH_SEPARATOR",
        ]
        .map(String::from)
        .to_vec();
        let mut names = BTreeSet::from([
            "STELLARR_IMAGE".to_string(),
            "STELLARR_TRACK".to_string(),
            "STELLARR_VERSION".to_string(),
        ]);
        for file in files {
            let content = fs::read(path.join(file)).unwrap_or_default();
            let mut rest = content.as_slice();
            while let Some(at) = rest.windows(2).position(|w| w == b"${") {
                rest = &rest[at + 2..];
                let first = rest.first().copied().unwrap_or(0);
                if first.is_ascii_alphabetic() || first == b'_' {
                    let end = rest
                        .iter()
                        .position(|b| !(b.is_ascii_alphanumeric() || *b == b'_'))
                        .unwrap_or(rest.len());
                    names.insert(String::from_utf8_lossy(&rest[..end]).into_owned());
                }
            }
        }
        let ours: Vec<&str> = KEYS
            .iter()
            .map(|k| k.name())
            .chain(["STELLARR_IMAGE", "STELLARR_TRACK", "STELLARR_VERSION"])
            .collect();
        let mut ignored = String::new();
        for name in names {
            if std::env::var_os(&name).is_none() {
                continue;
            }
            if !ours.contains(&name.as_str()) {
                ignored.push(' ');
                ignored.push_str(&name);
            }
            self.removed_env.push(name);
        }
        if !ignored.is_empty() {
            self.ui.warn(
                &format!("Ignored from the environment, as Compose reads .env:{ignored}"),
                NONE,
            );
        }
    }

    // ---- Starting ----

    fn activate(&mut self) -> Step {
        let (ok, log) = sys::logged(self.compose(&["up", "-d", "--no-build", "--pull", "never"]));
        if !ok {
            self.ui
                .fail("Docker Compose could not start the container", NONE);
            self.ui.block(Fd::Err, &log);
            let logs = format!("All logs: {}", self.compose_command("logs"));
            self.ui.text(
                Fd::Err,
                &[
                    "",
                    "The saved configuration is in place. Fix the cause and run the installer again.",
                    &logs,
                ],
            );
            return Err(Stop(1));
        }
        if self.owns_container && self.container.running {
            self.ui.ok("Brought the container up to date", NONE);
        } else {
            self.ui.ok("Started the container", NONE);
        }
        self.wait_until_up()
    }

    fn compose(&self, args: &[&str]) -> Command {
        self.compose_at(&self.install_dir, args)
    }
}

fn rebase_model_paths(value: &mut serde_json::Value, from: &Path, to: &Path) {
    use serde_json::Value;
    match value {
        Value::String(text) => {
            if let Ok(relative) = Path::new(text).strip_prefix(from) {
                *text = to.join(relative).to_string_lossy().into_owned();
            }
        }
        Value::Array(items) => {
            for item in items {
                rebase_model_paths(item, from, to);
            }
        }
        Value::Object(entries) => {
            for item in entries.values_mut() {
                rebase_model_paths(item, from, to);
            }
        }
        _ => {}
    }
}
