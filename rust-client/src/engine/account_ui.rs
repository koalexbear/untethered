//! App state and actions for the matrix-sdk engine's account screens: emoji
//! verification, recovery setup and reset, devices, sign-out and browser
//! sign-in. The screens themselves are in `view/sdk_account.rs`.
use super::{
    BrowserMethod, Command, DeviceInfo, Failure, LoginOptions, Recovery, Stage, Verification,
};
use crate::{App, Msg, account};
use iced::Task;
use std::collections::BTreeMap;

/// Requests these screens are waiting for, by command id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Job {
    VerifyDevice,
    SetUpRecovery,
    ChangeRecoveryKey,
    ResetEncryption,
    Devices,
    DeleteDevice(String),
    Logout,
    LoginOptions(String),
    BrowserLogin,
}

#[derive(Default)]
pub struct AccountUi {
    /// The account has cross-signing keys another device can verify against.
    pub identity: bool,
    pub recovery: Recovery,
    pub verification: Option<Verification>,
    /// Shown once, then dropped.
    pub recovery_key: Option<String>,
    pub key_copied: bool,
    pub devices: Option<Vec<DeviceInfo>>,
    /// The device whose sign-out form is open.
    pub removing: Option<String>,
    /// Account password for actions that need it; cleared after each.
    pub password: String,
    pub resetting: bool,
    pub signing_out: bool,
    /// Sign-in options and the homeserver (as typed) they belong to.
    pub options: Option<(String, LoginOptions)>,
    pub browser: bool,
    pub note: String,
    pub jobs: BTreeMap<i64, Job>,
    pub profile: super::profile::Ui,
}

impl AccountUi {
    pub fn busy(&self, job: &Job) -> bool {
        self.jobs.values().any(|j| j == job)
    }
}

#[derive(Clone)]
pub enum Action {
    Profile(super::profile::Action),
    VerifyWithDevice,
    AcceptVerification,
    EmojiMatch(bool),
    /// Decline a request, or stop a running verification.
    CancelVerification,
    /// Dismiss a finished or cancelled verification.
    CloseVerification,
    Password(String),
    SetUpRecovery,
    ChangeRecoveryKey,
    ShowReset(bool),
    ResetEncryption,
    CopyRecoveryKey,
    SavedRecoveryKey,
    RefreshDevices,
    AskRemove(Option<String>),
    RemoveDevice,
    AskSignOut(bool),
    SignOut,
    LoginOptions,
    BrowserLogin(BrowserMethod),
    CancelBrowserLogin,
}

impl App {
    fn job(&mut self, job: Job, make: impl FnOnce(i64) -> Command) {
        if self.sdk_ui.busy(&job) {
            return;
        }
        self.seq += 1;
        if self.connected
            && let Some(sdk) = &self.sdk
            && sdk.try_send(make(self.seq)).is_ok()
        {
            self.sdk_ui.jobs.insert(self.seq, job);
            self.sdk_ui.note.clear();
        } else {
            self.sdk_ui.note = "Could not queue the request. Try again.".into();
        }
    }

    fn signal(&self, command: Command) {
        if let Some(sdk) = &self.sdk {
            let _ = sdk.try_send(command);
        }
    }

    fn take_password(&mut self) -> Option<String> {
        if self.sdk_ui.password.is_empty() {
            self.sdk_ui.note = "Enter your account password first.".into();
            None
        } else {
            Some(std::mem::take(&mut self.sdk_ui.password))
        }
    }

    pub(crate) fn sdk_action(&mut self, action: Action) -> Task<Msg> {
        match action {
            Action::Profile(action) => return self.profile_action(action),
            Action::VerifyWithDevice if self.logged_in && !self.verified => {
                self.job(Job::VerifyDevice, |id| Command::VerifyDevice { id })
            }
            Action::AcceptVerification => self.signal(Command::AcceptVerification),
            Action::EmojiMatch(matches) => self.signal(Command::ConfirmEmoji { matches }),
            Action::CancelVerification => self.signal(Command::CancelVerification),
            Action::CloseVerification => {
                if self
                    .sdk_ui
                    .verification
                    .as_ref()
                    .is_some_and(|v| matches!(v.stage, Stage::Done | Stage::Cancelled(_)))
                {
                    self.sdk_ui.verification = None;
                }
            }
            Action::Password(p) => self.sdk_ui.password = p,
            Action::SetUpRecovery if self.logged_in => {
                // A verified device holds the keys; only a new identity needs
                // the password.
                if self.verified {
                    self.job(Job::ChangeRecoveryKey, |id| Command::ChangeRecoveryKey {
                        id,
                    });
                } else if let Some(password) = self.take_password() {
                    self.job(Job::SetUpRecovery, |id| Command::SetUpRecovery {
                        id,
                        password,
                    });
                }
            }
            Action::ChangeRecoveryKey if self.verified => self.job(Job::ChangeRecoveryKey, |id| {
                Command::ChangeRecoveryKey { id }
            }),
            Action::ShowReset(open) => {
                self.sdk_ui.resetting = open;
                self.sdk_ui.password.clear();
            }
            Action::ResetEncryption if self.logged_in && self.sdk_ui.resetting => {
                if let Some(password) = self.take_password() {
                    self.job(Job::ResetEncryption, |id| Command::ResetEncryption {
                        id,
                        password,
                    });
                }
            }
            Action::CopyRecoveryKey => {
                if let Some(key) = &self.sdk_ui.recovery_key {
                    self.sdk_ui.key_copied = true;
                    return iced::clipboard::write(key.clone());
                }
            }
            Action::SavedRecoveryKey => {
                self.sdk_ui.recovery_key = None;
                self.sdk_ui.key_copied = false;
            }
            Action::RefreshDevices if self.logged_in => {
                self.job(Job::Devices, |id| Command::Devices { id })
            }
            Action::AskRemove(device) => {
                self.sdk_ui.removing = device;
                self.sdk_ui.password.clear();
            }
            Action::RemoveDevice => {
                if let Some(device) = self.sdk_ui.removing.clone()
                    && let Some(password) = self.take_password()
                {
                    let target = device.clone();
                    self.job(Job::DeleteDevice(device), |id| Command::DeleteDevice {
                        id,
                        device: target,
                        password,
                    });
                }
            }
            Action::AskSignOut(open) => self.sdk_ui.signing_out = open,
            Action::SignOut if self.logged_in && self.sdk_ui.signing_out => {
                self.job(Job::Logout, |id| Command::Logout { id })
            }
            Action::LoginOptions if !self.logged_in => match account::homeserver(&self.server) {
                Some(url) => {
                    let homeserver = url.clone();
                    self.job(Job::LoginOptions(url), |id| Command::LoginOptions {
                        id,
                        homeserver,
                    });
                }
                None => {
                    self.status = "Enter your homeserver (for example, matrix.org) first.".into()
                }
            },
            Action::BrowserLogin(method) if !self.logged_in && !self.sdk_ui.browser => {
                if let Some((homeserver, _)) = self.sdk_ui.options.clone() {
                    self.sdk_ui.browser = true;
                    self.status = "Continue in your browser…".into();
                    self.job(Job::BrowserLogin, |id| Command::BrowserLogin {
                        id,
                        homeserver,
                        method,
                    });
                    if !self.sdk_ui.busy(&Job::BrowserLogin) {
                        self.sdk_ui.browser = false;
                    }
                }
            }
            Action::CancelBrowserLogin if self.sdk_ui.browser => {
                self.signal(Command::CancelBrowserLogin)
            }
            _ => {}
        }
        Task::none()
    }

    pub(crate) fn sdk_verification(&mut self, view: Option<Verification>) {
        let finished_now = view
            .as_ref()
            .is_some_and(|v| v.stage == Stage::Done && !v.incoming);
        self.sdk_ui.verification = view;
        if finished_now {
            self.sdk_ui.note.clear();
        }
    }

    pub(crate) fn sdk_login_options(&mut self, options: LoginOptions) {
        // Attach them to the homeserver as typed when the request was made.
        let typed = self.sdk_ui.jobs.values().find_map(|j| match j {
            Job::LoginOptions(server) => Some(server.clone()),
            _ => None,
        });
        if let Some(typed) = typed {
            self.sdk_ui.options = Some((typed, options));
        }
    }

    /// Sign-in options for the homeserver currently in the form.
    pub(crate) fn sdk_current_options(&self) -> Option<&LoginOptions> {
        let (server, options) = self.sdk_ui.options.as_ref()?;
        (account::homeserver(&self.server).as_ref() == Some(server)).then_some(options)
    }

    /// Requests from these screens. Returns false if `id` isn't one of them.
    pub(crate) fn sdk_done(&mut self, id: i64, result: Result<(), Failure>) -> bool {
        let Some(job) = self.sdk_ui.jobs.remove(&id) else {
            return false;
        };
        let ui = &mut self.sdk_ui;
        let password = |f: Failure, otherwise: &'static str| match f {
            Failure::WrongPassword => "The homeserver didn’t accept that password.",
            Failure::Unreachable => "Couldn’t reach the homeserver. Try again when it’s reachable.",
            _ => otherwise,
        };
        match (job, result) {
            (Job::VerifyDevice, Ok(())) => {}
            (Job::VerifyDevice, Err(Failure::NoIdentity)) => {
                ui.note = "This account has no encryption keys yet, so there’s nothing to verify against. Set up recovery instead.".into()
            }
            (Job::VerifyDevice, Err(f)) => {
                ui.note = password(f, "Couldn’t ask your other devices. Try again.").into()
            }
            (Job::SetUpRecovery | Job::ChangeRecoveryKey | Job::ResetEncryption, Ok(())) => {
                ui.resetting = false;
            }
            (Job::SetUpRecovery, Err(Failure::NoIdentity)) => {
                ui.note = "This account already has encryption keys. Verify with another device or use your recovery key.".into()
            }
            (Job::ResetEncryption, Err(Failure::NeedsBrowser)) => {
                ui.note = "Your homeserver approves this reset in a browser, which Untethered can’t do yet.".into()
            }
            (Job::SetUpRecovery | Job::ResetEncryption, Err(f)) => {
                ui.note = password(f, "That didn’t work. Nothing was changed; try again.").into()
            }
            (Job::ChangeRecoveryKey, Err(f)) => {
                ui.note = password(f, "Couldn’t create a new recovery key. Try again.").into()
            }
            (Job::Devices, Ok(())) => {}
            (Job::Devices, Err(f)) => {
                ui.note = password(f, "Couldn’t load your devices.").into()
            }
            (Job::DeleteDevice(device), Ok(())) => {
                ui.removing = None;
                ui.note = format!("Signed out {device}.");
            }
            (Job::DeleteDevice(_), Err(f)) => {
                ui.note = password(f, "Couldn’t sign that device out.").into()
            }
            (Job::Logout, _) => ui.signing_out = false,
            (Job::LoginOptions(_), Ok(())) => {}
            (Job::LoginOptions(_), Err(_)) => {
                self.status =
                    "No Matrix homeserver answered at that address. Check it and try again.".into()
            }
            (Job::BrowserLogin, result) => {
                ui.browser = false;
                self.status = match result {
                    Ok(()) => String::new(),
                    Err(Failure::Cancelled) => "Browser sign-in cancelled.".into(),
                    Err(Failure::Forbidden) => {
                        "The homeserver didn’t accept the browser sign-in.".into()
                    }
                    Err(Failure::NoSlidingSync) => "This homeserver doesn’t offer Simplified Sliding Sync, which the matrix-sdk engine needs. Start without UNTETHERED_ENGINE to use gomuks.".into(),
                    Err(Failure::Unreachable) => {
                        "No Matrix homeserver answered at that address. Check it and try again."
                            .into()
                    }
                    Err(_) => "Browser sign-in failed. Try again.".into(),
                };
            }
        }
        true
    }
}
