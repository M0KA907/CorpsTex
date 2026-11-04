//! Independent app-bound project registries; context and authorization never cross runtimes.
use crate::{
    projects::{Registry, Runtime, View},
    runtime::Error,
};
pub struct Runtimes {
    codex: Registry,
    opencode: Registry,
}
impl Default for Runtimes {
    fn default() -> Self {
        Self {
            codex: Registry::default(),
            opencode: Registry::opencode(),
        }
    }
}
impl Runtimes {
    pub fn get(&self, runtime: Option<Runtime>) -> &Registry {
        match runtime.unwrap_or_default() {
            Runtime::Codex => &self.codex,
            Runtime::Opencode => &self.opencode,
        }
    }
    pub fn view(&self, window: &str) -> Result<View, Error> {
        let mut view = self.codex.view(window)?;
        let other = self.opencode.view(window)?;
        view.projects.extend(other.projects);
        view.approvals.extend(other.approvals);
        view.approvals.sort_by_key(|a| a.ticket);
        view.exit_projects.extend(other.exit_projects);
        view.exit_requested |= other.exit_requested;
        Ok(view)
    }
    pub fn request_exit(&self) -> Result<bool, Error> {
        let codex = self.codex.request_exit()?;
        let opencode = self.opencode.request_exit()?;
        Ok(codex || opencode)
    }
    pub fn cancel_exit(&self) {
        self.codex.cancel_exit();
        self.opencode.cancel_exit();
    }
    pub fn detach_window(&self, window: &str) {
        self.codex.detach_window(window);
        self.opencode.detach_window(window);
    }
    pub fn shutdown(&self, confirmed: bool) -> Result<(), Error> {
        if !confirmed && (self.codex.has_projects()? || self.opencode.has_projects()?) {
            return Err(Error::ConfirmationRequired);
        }
        self.codex.shutdown(confirmed)?;
        self.opencode.shutdown(confirmed)
    }
}
