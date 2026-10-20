#[path = "../src/inject.rs"]
mod inject;
use std::{process::Command, time::{Duration, Instant}};

#[test]
#[ignore = "requires isolated Xvfb and VOICETYPE_INJECTION_PROBE=1"]
fn real_sakura_readback() {
    assert_eq!(std::env::var("VOICETYPE_INJECTION_PROBE").as_deref(), Ok("1"));
    assert_ne!(std::env::var("DISPLAY").unwrap(), ":0", "isolated display required");
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).is_test(true).try_init();
    let dir = std::env::temp_dir().join(format!("voicetype-sakura-probe-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let output = dir.join("received.txt");
    let receiver = dir.join("receiver.py");
    let expected = "真实 Sakura 中文输入 café 123";
    // A raw-mode receiver, not a shell: pasted bytes cannot execute commands.
    std::fs::write(&receiver, format!("import os,tty,select,time\ntty.setraw(0)\ndata=b''\nend=time.time()+10\nwhile time.time()<end:\n if select.select([0],[],[],0.1)[0]:\n  data+=os.read(0,4096)\n  open({:?},'wb').write(data)\n", output.to_str().unwrap())).unwrap();
    let mut terminal = Command::new("sakura").args(["--config-file", dir.join("sakura.conf").to_str().unwrap(), "-e", "python3", receiver.to_str().unwrap()]).spawn().unwrap();
    let pid = terminal.id().to_string();
    let deadline = Instant::now()+Duration::from_secs(5);
    let id = loop {
        let result = Command::new("xdotool").args(["search", "--onlyvisible", "--all", "--pid", &pid, "--class", "Sakura"]).output().unwrap();
        if result.status.success() {
            let ids = String::from_utf8(result.stdout).unwrap();
            if let Some(id) = ids.lines().last() { break id.to_string(); }
        }
        assert!(Instant::now()<deadline, "Sakura did not map");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(Command::new("xdotool").args(["windowfocus", &id]).status().unwrap().success());
    std::thread::sleep(Duration::from_millis(300));
    // Focus its unnamed input child, matching the observed user's hierarchy.
    let tree = Command::new("xwininfo").args(["-id", &id, "-tree"]).output().unwrap();
    let tree = String::from_utf8(tree.stdout).unwrap();
    if let Some(child) = tree.lines().find(|line| line.contains("(has no name)") && line.trim().starts_with("0x")) {
        let child = child.split_whitespace().next().unwrap();
        assert!(Command::new("xdotool").args(["windowfocus", child]).status().unwrap().success());
    }
    inject::inject(expected, false);
    let actual = std::fs::read_to_string(&output).unwrap_or_default();
    let _ = terminal.kill(); let _ = terminal.wait();
    assert_eq!(actual, expected, "actual bytes received from Sakura PTY");
    eprintln!("PASS real Sakura PTY readback: {actual:?}");
}
