#![forbid(unsafe_code)]

use tellus::GroundingPort;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let Some(mode) = args.next() else {
        return Err(usage().into());
    };
    if mode == "--help" || mode == "-h" {
        println!("{}", usage());
        return Ok(());
    }
    if mode == "capabilities" {
        if args.next().is_some() {
            return Err(usage().into());
        }
        let value = tellus::CAPABILITIES;
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "schema": value.schema, "peer": value.peer,
                "interactive": value.interactive, "headless": value.headless
            }))?
        );
        return Ok(());
    }
    if mode != "interactive" && mode != "headless" {
        return Err(usage().into());
    }
    let endpoint = args
        .next()
        .ok_or_else(|| std::io::Error::other(usage()))?
        .to_string_lossy()
        .parse::<std::net::SocketAddr>()?;
    let token = match args.next() {
        Some(flag) if flag == "--token-file" => {
            let path = args.next().ok_or_else(|| std::io::Error::other(usage()))?;
            let value = std::fs::read_to_string(path)?;
            Some(value.trim_end().to_owned())
        }
        Some(_) => return Err(usage().into()),
        None => None,
    };
    if args.next().is_some() {
        return Err(usage().into());
    }
    let port =
        tellus_local_client::LocalPort::new(endpoint, token, std::time::Duration::from_secs(5))
            .map_err(|error| std::io::Error::other(format!("{error:?}")))?;
    if mode == "interactive" {
        interactive(tellus::Component::new(port))
    } else {
        headless(port)
    }
}

fn interactive(mut component: tellus::Component) -> Result<(), Box<dyn std::error::Error>> {
    use crossterm::event::{self, Event};
    use std::time::Duration;
    ratatui::run(|terminal| -> Result<(), Box<dyn std::error::Error>> {
        loop {
            component.poll();
            terminal.draw(|frame| component.draw_in(frame, frame.area(), true))?;
            if event::poll(Duration::from_millis(50))?
                && let Event::Key(key) = event::read()?
                && let Some(input) = tellus::translate_key(key)
                && component.update(input, true) == tellus::Lifecycle::CloseRequested
            {
                break;
            }
        }
        Ok(())
    })
}

fn headless(mut port: tellus_local_client::LocalPort) -> Result<(), Box<dyn std::error::Error>> {
    match port.observe(tellus::OBSERVATION_LIMIT) {
        Ok(value) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "version": 1, "mode": "headless", "peer": "grounding",
                    "capability": "grounding.observe.v1", "complete": value.complete,
                    "status": value.status, "entries": value.entries.iter().map(|entry| serde_json::json!({
                        "load_id": entry.load_id, "state": entry.state,
                        "freshness": entry.freshness, "sha256": entry.sha256
                    })).collect::<Vec<_>>(), "truncated": value.truncated
                }))?
            );
            if value.complete {
                Ok(())
            } else {
                Err(std::io::Error::other("partial Grounding observation").into())
            }
        }
        Err(error) => Err(std::io::Error::other(format!("{error:?}")).into()),
    }
}

fn usage() -> &'static str {
    "usage: tellus capabilities | tellus <interactive|headless> LOOPBACK_HOST:PORT [--token-file PATH]"
}
