//! Screen sizes: the standard ones, and the main screen's.

/// A standard size and its usual names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Its names ("Full HD"); not translated.
    pub name: &'static str,
}

/// The standard sizes, largest first.
pub const STANDARD: &[Size] = &[
    Size {
        width: 7680,
        height: 4320,
        name: "8K",
    },
    Size {
        width: 3840,
        height: 2160,
        name: "4K UHD",
    },
    Size {
        width: 2560,
        height: 1440,
        name: "QHD, 2K",
    },
    Size {
        width: 1920,
        height: 1080,
        name: "Full HD",
    },
    Size {
        width: 1600,
        height: 900,
        name: "HD+",
    },
    Size {
        width: 1280,
        height: 720,
        name: "HD",
    },
    Size {
        width: 1024,
        height: 768,
        name: "XGA",
    },
    Size {
        width: 960,
        height: 540,
        name: "qHD",
    },
    Size {
        width: 800,
        height: 600,
        name: "SVGA",
    },
    Size {
        width: 640,
        height: 480,
        name: "VGA",
    },
    Size {
        width: 480,
        height: 320,
        name: "HVGA",
    },
    Size {
        width: 320,
        height: 240,
        name: "QVGA",
    },
];

/// What the main screen runs at now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Primary {
    /// Its size in pixels, turned when the screen stands on its side.
    pub size: (u32, u32),
    /// Its refresh rate in whole hertz (159.96 Hz reads 160), when known.
    pub refresh_hz: Option<u32>,
    /// Its adaptive sync (VRR, `FreeSync`) setting, where the desktop says:
    /// only KDE does, and only for a screen that has it.
    pub vrr: Option<Vrr>,
}

/// KDE's adaptive sync policy for a screen (`vrrPolicy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vrr {
    /// Never on.
    Never,
    /// Always on.
    Always,
    /// On for fullscreen games and video.
    Automatic,
}

/// The main screen: KDE's highest-priority output (`kscreen-doctor`), else
/// the output `xrandr` calls primary.
#[must_use]
pub fn primary() -> Option<Primary> {
    let run = |program: &str, args: &[&str]| {
        std::process::Command::new(program)
            .args(args)
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    run("kscreen-doctor", &["-j"])
        .and_then(|j| from_kscreen(&j))
        .or_else(|| run("xrandr", &["--query"]).and_then(|t| from_xrandr(&t)))
}

/// The main screen's size in pixels ([`primary`]).
#[must_use]
pub fn primary_size() -> Option<(u32, u32)> {
    primary().map(|p| p.size)
}

/// A refresh rate in whole hertz.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn whole_hz(rate: f64) -> Option<u32> {
    (1.0..10_000.0).contains(&rate).then(|| rate.round() as u32)
}

/// The enabled output with the lowest priority number (1 = primary), in
/// its current mode, turned when the output is.
fn from_kscreen(json: &str) -> Option<Primary> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let outputs = v.get("outputs")?.as_array()?;
    let out = outputs
        .iter()
        .filter(|o| o.get("enabled").and_then(serde_json::Value::as_bool) == Some(true))
        .min_by_key(|o| {
            o.get("priority")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(u64::MAX)
        })?;
    let current = out.get("currentModeId")?;
    let mode = out
        .get("modes")?
        .as_array()?
        .iter()
        .find(|m| m.get("id") == Some(current))?;
    let size = mode.get("size")?;
    let dim = |k: &str| {
        size.get(k)
            .and_then(serde_json::Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
    };
    let (w, h) = (dim("width")?, dim("height")?);
    // 2 = left, 8 = right: the screen stands on its side.
    let turned = matches!(
        out.get("rotation").and_then(serde_json::Value::as_u64),
        Some(2 | 8)
    );
    // KDE lists a policy only for a screen that can do adaptive sync.
    let vrr = match out.get("vrrPolicy").and_then(serde_json::Value::as_u64) {
        Some(0) => Some(Vrr::Never),
        Some(1) => Some(Vrr::Always),
        Some(2) => Some(Vrr::Automatic),
        _ => None,
    };
    Some(Primary {
        size: if turned { (h, w) } else { (w, h) },
        refresh_hz: mode
            .get("refreshRate")
            .and_then(serde_json::Value::as_f64)
            .and_then(whole_hz),
        vrr,
    })
}

/// `DP-1 connected primary 3440x1440+3440+0 …` and, under it, its modes
/// with the current one starred (`3440x1440  159.86*+`); without an output
/// marked primary, which X11 desktops other than KDE often leave unset, the
/// first connected output that is on.
fn from_xrandr(text: &str) -> Option<Primary> {
    let size_of = |line: &str| -> Option<(u32, u32)> {
        let geometry = line
            .split_whitespace()
            .find(|w| w.contains('x') && w.contains('+'))?;
        let (size, _) = geometry.split_once('+')?;
        let (w, h) = size.split_once('x')?;
        Some((w.parse().ok()?, h.parse().ok()?))
    };
    let lines: Vec<&str> = text.lines().collect();
    let at = lines
        .iter()
        .position(|l| l.contains(" connected primary ") && size_of(l).is_some())
        .or_else(|| {
            lines
                .iter()
                .position(|l| l.contains(" connected ") && size_of(l).is_some())
        })?;
    let refresh_hz = lines[at + 1..]
        .iter()
        .take_while(|l| l.starts_with(char::is_whitespace))
        .flat_map(|l| l.split_whitespace())
        .find(|w| w.contains('*'))
        .and_then(|w| w.trim_end_matches(['*', '+']).parse::<f64>().ok())
        .and_then(whole_hz);
    Some(Primary {
        size: size_of(lines[at])?,
        refresh_hz,
        vrr: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kde_names_the_main_screen_by_priority() {
        let json = r#"{"outputs":[
          {"name":"HDMI-A-1","enabled":true,"priority":2,"rotation":1,"currentModeId":"1",
           "modes":[{"id":"1","size":{"width":3440,"height":1440}}]},
          {"name":"DP-2","enabled":true,"priority":3,"rotation":1,"currentModeId":"34",
           "modes":[{"id":"34","size":{"width":2560,"height":1080}}]},
          {"name":"DP-1","enabled":true,"priority":1,"rotation":1,"currentModeId":"89",
           "vrrPolicy":0,
           "modes":[{"id":"88","size":{"width":1920,"height":1080}},
                    {"id":"89","refreshRate":159.96200561523438,"size":{"width":3440,"height":1440}}]},
          {"name":"DP-3","enabled":false,"priority":0,"currentModeId":"1",
           "modes":[{"id":"1","size":{"width":800,"height":600}}]}]}"#;
        assert_eq!(
            from_kscreen(json),
            Some(Primary {
                size: (3440, 1440),
                refresh_hz: Some(160),
                vrr: Some(Vrr::Never),
            })
        );
        let turned = json.replace(
            r#""priority":1,"rotation":1"#,
            r#""priority":1,"rotation":2"#,
        );
        assert_eq!(from_kscreen(&turned).map(|p| p.size), Some((1440, 3440)));
        // A screen without adaptive sync has no policy at all.
        let fixed = json.replace(r#""vrrPolicy":0,"#, "");
        assert_eq!(from_kscreen(&fixed).and_then(|p| p.vrr), None);
    }

    #[test]
    fn xrandr_names_the_primary_output() {
        let text = "Screen 0: minimum 16 x 16, current 9440 x 1440, maximum 32767 x 32767\n\
            DP-1 connected primary 3440x1440+3440+0 (normal left inverted right x axis y axis) 800mm x 334mm\n\
            \x20  3440x1440    159.86*+  99.98\n\
            \x20  2560x1080     59.98\n\
            DP-2 connected 2560x1080+6880+0 (normal left inverted right x axis y axis) 798mm x 334mm\n\
            \x20  2560x1080     74.99*+\n";
        assert_eq!(
            from_xrandr(text),
            Some(Primary {
                size: (3440, 1440),
                refresh_hz: Some(160),
                vrr: None,
            })
        );
    }

    #[test]
    fn xrandr_without_a_primary_names_the_first_output_that_is_on() {
        let text = "Screen 0: minimum 8 x 8, current 1920 x 1080, maximum 32767 x 32767\n\
            eDP-1 connected (normal left inverted right x axis y axis)\n\
            HDMI-1 connected 1920x1080+0+0 (normal left inverted right x axis y axis) 527mm x 296mm\n\
            DP-1 disconnected (normal left inverted right x axis y axis)\n";
        assert_eq!(from_xrandr(text).map(|p| p.size), Some((1920, 1080)));
        assert_eq!(from_xrandr(text).and_then(|p| p.refresh_hz), None);
        assert_eq!(from_xrandr("DP-1 disconnected\n"), None);
    }

    #[test]
    fn the_standard_sizes_go_from_largest_to_smallest() {
        assert!(
            STANDARD
                .windows(2)
                .all(|w| w[0].width * w[0].height > w[1].width * w[1].height)
        );
    }
}
