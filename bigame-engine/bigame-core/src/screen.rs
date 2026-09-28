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

/// The main screen's size in pixels: KDE's highest-priority output
/// (`kscreen-doctor`), else the output `xrandr` calls primary.
#[must_use]
pub fn primary_size() -> Option<(u32, u32)> {
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

/// The enabled output with the lowest priority number (1 = primary), in
/// its current mode, turned when the output is.
fn from_kscreen(json: &str) -> Option<(u32, u32)> {
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
    Some(if turned { (h, w) } else { (w, h) })
}

/// `DP-1 connected primary 3440x1440+3440+0 …`; without an output marked
/// primary, which X11 desktops other than KDE often leave unset, the first
/// connected output that is on.
fn from_xrandr(text: &str) -> Option<(u32, u32)> {
    let size_of = |line: &str| -> Option<(u32, u32)> {
        let geometry = line
            .split_whitespace()
            .find(|w| w.contains('x') && w.contains('+'))?;
        let (size, _) = geometry.split_once('+')?;
        let (w, h) = size.split_once('x')?;
        Some((w.parse().ok()?, h.parse().ok()?))
    };
    text.lines()
        .find(|l| l.contains(" connected primary "))
        .and_then(size_of)
        .or_else(|| {
            text.lines()
                .filter(|l| l.contains(" connected "))
                .find_map(size_of)
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
           "modes":[{"id":"88","size":{"width":1920,"height":1080}},
                    {"id":"89","size":{"width":3440,"height":1440}}]},
          {"name":"DP-3","enabled":false,"priority":0,"currentModeId":"1",
           "modes":[{"id":"1","size":{"width":800,"height":600}}]}]}"#;
        assert_eq!(from_kscreen(json), Some((3440, 1440)));
        let turned = json.replace(
            r#""priority":1,"rotation":1"#,
            r#""priority":1,"rotation":2"#,
        );
        assert_eq!(from_kscreen(&turned), Some((1440, 3440)));
    }

    #[test]
    fn xrandr_names_the_primary_output() {
        let text = "Screen 0: minimum 16 x 16, current 9440 x 1440, maximum 32767 x 32767\n\
            DP-1 connected primary 3440x1440+3440+0 (normal left inverted right x axis y axis) 800mm x 334mm\n\
            DP-2 connected 2560x1080+6880+0 (normal left inverted right x axis y axis) 798mm x 334mm\n";
        assert_eq!(from_xrandr(text), Some((3440, 1440)));
    }

    #[test]
    fn xrandr_without_a_primary_names_the_first_output_that_is_on() {
        let text = "Screen 0: minimum 8 x 8, current 1920 x 1080, maximum 32767 x 32767\n\
            eDP-1 connected (normal left inverted right x axis y axis)\n\
            HDMI-1 connected 1920x1080+0+0 (normal left inverted right x axis y axis) 527mm x 296mm\n\
            DP-1 disconnected (normal left inverted right x axis y axis)\n";
        assert_eq!(from_xrandr(text), Some((1920, 1080)));
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
