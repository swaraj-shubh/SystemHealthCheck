//! Parsers turning raw command output into structured data.
//!
//! Every parser is a pure function over text so it can be tested with mocked
//! output. Unknown fields are always kept (`fields` vectors) so nothing the
//! command returned is lost from the structured view.

pub mod bench;
pub mod boot;
pub mod hardware;
pub mod kernel;
pub mod packages;
pub mod power;
pub mod storage;

use std::collections::BTreeMap;

/// `Key: Value` lines (first colon), trimmed, in order. Lines without a colon
/// or with an empty key are skipped.
pub fn parse_kv(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            let k = k.trim();
            (!k.is_empty()).then(|| (k.to_string(), v.trim().to_string()))
        })
        .collect()
}

/// Lookup helper over key/value vectors (case-insensitive key match).
pub fn kv_get<'a>(kv: &'a [(String, String)], key: &str) -> Option<&'a str> {
    kv.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str()).filter(|v| !v.is_empty())
}

/// Leading number of a string, ignoring thousands separators: "12,345 (6 TB)" -> 12345.
pub fn leading_num(s: &str) -> Option<f64> {
    let s = s.trim().trim_start_matches('+');
    let end = s.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ',' || c == '-')).unwrap_or(s.len());
    let n = s[..end].replace(',', "");
    n.parse().ok()
}

/// Parse a systemd time span: "1min 2.345s", "345ms", "7.353s", "2h 3min".
pub fn parse_systemd_time(s: &str) -> Option<f64> {
    let mut total = 0.0;
    let mut any = false;
    for tok in s.split_whitespace() {
        let idx = tok.find(|c: char| c.is_ascii_alphabetic())?;
        let (n, unit) = tok.split_at(idx);
        let n: f64 = n.parse().ok()?;
        total += match unit {
            "us" | "µs" => n / 1e6,
            "ms" => n / 1e3,
            "s" => n,
            "min" => n * 60.0,
            "h" => n * 3600.0,
            "d" => n * 86400.0,
            _ => return None,
        };
        any = true;
    }
    any.then_some(total)
}

/// Table whose header is followed by a `---- ----` dashes line (nvme list).
/// Column boundaries come from the dash groups.
pub fn parse_dashed_table(text: &str) -> Vec<BTreeMap<String, String>> {
    let lines: Vec<&str> = text.lines().collect();
    let Some(di) = lines.iter().position(|l| l.trim_start().starts_with("---")) else { return Vec::new() };
    if di == 0 {
        return Vec::new();
    }
    let dashes = lines[di];
    let mut cols = Vec::new();
    let mut start = None;
    for (i, c) in dashes.char_indices().chain(std::iter::once((dashes.len(), ' '))) {
        match (c == '-', start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                cols.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    let slice = |l: &str, (s, e): (usize, usize), last: bool| -> String {
        let end = if last { l.len() } else { e.min(l.len()) };
        l.get(s.min(l.len())..end).unwrap_or("").trim().to_string()
    };
    let header = lines[di - 1];
    let names: Vec<String> = cols.iter().enumerate().map(|(i, c)| slice(header, *c, i + 1 == cols.len())).collect();
    lines[di + 1..]
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| names.iter().cloned().zip(cols.iter().enumerate().map(|(i, c)| slice(l, *c, i + 1 == cols.len()))).collect())
        .collect()
}

/// Table with a left-aligned header row (nmcli, systemctl): columns start where
/// header words start. Each column runs to the start of the next.
pub fn parse_header_table(text: &str) -> Vec<BTreeMap<String, String>> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let Some(header) = lines.next() else { return Vec::new() };
    let mut starts = Vec::new();
    let mut prev_space = true;
    for (i, c) in header.char_indices() {
        if !c.is_whitespace() && prev_space {
            starts.push(i);
        }
        prev_space = c.is_whitespace();
    }
    // Header words containing single spaces ("UNIT FILE") form one column.
    let mut cols: Vec<usize> = Vec::new();
    for s in starts {
        let joined = cols.last().is_some_and(|&p| header[p..s].trim_end().len() + p + 1 == s);
        if !joined {
            cols.push(s);
        }
    }
    let names: Vec<String> = cols
        .iter()
        .enumerate()
        .map(|(i, &s)| header[s..cols.get(i + 1).copied().unwrap_or(header.len())].trim().to_string())
        .collect();
    lines
        .take_while(|l| !l.ends_with("listed.") && !l.ends_with("listed"))
        .map(|l| {
            names
                .iter()
                .enumerate()
                .map(|(i, n)| {
                    let s = cols[i].min(l.len());
                    let e = cols.get(i + 1).copied().unwrap_or(l.len()).min(l.len());
                    let e = if i + 1 == cols.len() { l.len() } else { e };
                    (n.clone(), l.get(s..e).unwrap_or("").trim().to_string())
                })
                .collect()
        })
        .collect()
}

/// One `Handle ...` block of dmidecode output.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DmiSection {
    pub title: String,
    pub fields: Vec<(String, String)>,
}

/// Generic dmidecode `-t` parser. List-valued fields (Characteristics, ...)
/// are joined with ", ".
pub fn parse_dmidecode(text: &str) -> Vec<DmiSection> {
    let mut out: Vec<DmiSection> = Vec::new();
    let mut expect_title = false;
    for line in text.lines() {
        if line.starts_with("Handle ") {
            expect_title = true;
            continue;
        }
        if expect_title {
            if !line.trim().is_empty() {
                out.push(DmiSection { title: line.trim().to_string(), fields: Vec::new() });
            }
            expect_title = false;
            continue;
        }
        let Some(sec) = out.last_mut() else { continue };
        if line.starts_with("\t\t") {
            if let Some((_, v)) = sec.fields.last_mut() {
                if !v.is_empty() {
                    v.push_str(", ");
                }
                v.push_str(line.trim());
            }
        } else if let Some(rest) = line.strip_prefix('\t') {
            if let Some((k, v)) = rest.split_once(':') {
                sec.fields.push((k.trim().to_string(), v.trim().to_string()));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systemd_time() {
        assert_eq!(parse_systemd_time("7.353s"), Some(7.353));
        assert_eq!(parse_systemd_time("345ms"), Some(0.345));
        assert_eq!(parse_systemd_time("1min 2.5s"), Some(62.5));
        assert_eq!(parse_systemd_time("abc"), None);
    }

    #[test]
    fn numbers() {
        assert_eq!(leading_num("12,345,678 (6.32 TB)"), Some(12345678.0));
        assert_eq!(leading_num("+52.0 C"), Some(52.0));
        assert_eq!(leading_num("N/A"), None);
    }

    #[test]
    fn header_table_handles_spaced_values() {
        let t = "DEVICE           TYPE      STATE                   CONNECTION      \nwlan0            wifi      connected               home            \nlo               loopback  connected (externally)  lo              \neth0             ethernet  unavailable             --              \n";
        let rows = parse_header_table(t);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1]["STATE"], "connected (externally)");
        assert_eq!(rows[2]["CONNECTION"], "--");
        let u = "UNIT FILE                               STATE   PRESET\nssh.service                             enabled enabled\n\n2 unit files listed.\n";
        let rows = parse_header_table(u);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["UNIT FILE"], "ssh.service");
    }

    #[test]
    fn dashed_table() {
        let t = "Node                  Generic               SN                   Model                                    Namespace  Usage                      Format           FW Rev  \n--------------------- --------------------- -------------------- ---------------------------------------- ---------- -------------------------- ---------------- --------\n/dev/nvme0n1          /dev/ng0n1            ABC123               WDC PC SN530 SDBPNPZ-512G-1006           0x1        512.11  GB / 512.11  GB    512   B +  0 B   21106000\n";
        let rows = parse_dashed_table(t);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["Model"], "WDC PC SN530 SDBPNPZ-512G-1006");
        assert_eq!(rows[0]["FW Rev"], "21106000");
        assert_eq!(rows[0]["Usage"], "512.11  GB / 512.11  GB");
    }
}
