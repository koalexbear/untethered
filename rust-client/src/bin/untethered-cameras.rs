//! Enumerate only the registry exposed by the desktop Camera portal. A
//! numeric object serial survives new client connections but not a hotplug.
#[cfg(target_os = "linux")]
mod linux {
    use pipewire as pw;
    use std::{
        cell::{Cell, RefCell},
        os::fd::AsFd,
        rc::Rc,
        time::{Duration, Instant},
    };

    pub fn enumerate() -> Result<Vec<(String, u64)>, &'static str> {
        let fd = std::io::stdin()
            .as_fd()
            .try_clone_to_owned()
            .map_err(|_| "Missing portal FD")?;
        pw::init();
        let err = |_| "Could not list portal cameras.";
        let main = pw::main_loop::MainLoopRc::new(None).map_err(err)?;
        let ctx = pw::context::ContextRc::new(&main, None).map_err(err)?;
        let core = ctx.connect_fd_rc(fd, None).map_err(err)?;
        let reg = core.get_registry().map_err(err)?;
        let found = Rc::new(RefCell::new(Vec::new()));
        let entries = found.clone();
        let _nodes = reg
            .add_listener_local()
            .global(move |g| {
                let Some(p) = &g.props else { return };
                if g.type_ != pw::types::ObjectType::Node
                    || p.get("media.class") != Some("Video/Source")
                {
                    return;
                }
                if let Some(serial) = p.get("object.serial").and_then(|v| v.parse().ok()) {
                    let label = p
                        .get("node.description")
                        .or_else(|| p.get("node.nick"))
                        .or_else(|| p.get("node.name"))
                        .unwrap_or("Camera");
                    let label: String =
                        label.chars().filter(|c| !c.is_control()).take(96).collect();
                    entries.borrow_mut().push((label, serial));
                }
            })
            .register();
        let seq = core.sync(0).map_err(err)?;
        let done = Rc::new(Cell::new(false));
        let signal = done.clone();
        let _done = core
            .add_listener_local()
            .done(move |id, s| {
                if id == pw::core::PW_ID_CORE && s == seq {
                    signal.set(true)
                }
            })
            .register();
        let start = Instant::now();
        while !done.get() {
            if start.elapsed() > Duration::from_secs(3) {
                return Err("Camera list timed out.");
            }
            main.loop_().iterate(Duration::from_millis(75));
        }
        let mut entries = found.borrow_mut();
        entries.sort();
        entries.dedup();
        Ok(std::mem::take(&mut *entries))
    }
}
fn main() {
    #[cfg(target_os = "linux")]
    match linux::enumerate() {
        Ok(cameras) => println!("{}", serde_json::json!(cameras)),
        Err(_) => std::process::exit(1),
    }
}
