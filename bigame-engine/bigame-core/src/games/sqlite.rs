//! Rows of one table of an `SQLite` database, read straight from the file:
//! Lutris keeps its games' titles, runners and directories in `pga.db`, and
//! its YAML files (since 0.5.22) carry none of them.
//!
//! A reader of the file format (<https://sqlite.org/fileformat2.html>), not
//! a database: table b-trees, records and overflow pages, nothing else. It
//! never writes and never locks, so Lutris running meanwhile is not
//! disturbed; what a running Lutris still holds in its write-ahead log is
//! read on the next scan after it checkpoints. Anything unexpected yields no
//! rows.

use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Larger than any game launcher's database; a bigger file is not read.
const MAX_FILE: u64 = 64 * 1024 * 1024;

/// A column value.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Value {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob,
}

impl Value {
    pub(super) fn text(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s),
            _ => None,
        }
    }

    pub(super) fn int(&self) -> Option<i64> {
        match self {
            Self::Int(n) => Some(*n),
            _ => None,
        }
    }
}

/// One row, by column name.
pub(super) type Row = HashMap<String, Value>;

/// The rows of `table` in the database at `path`; empty when the file is
/// missing, too large, or not something this can read.
pub(super) fn read_table(path: &Path, table: &str) -> Vec<Row> {
    let too_big = std::fs::metadata(path).map_or(true, |m| m.len() > MAX_FILE);
    if too_big {
        return Vec::new();
    }
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    match rows(&bytes, table) {
        Ok(rows) => rows,
        Err(reason) => {
            tracing::debug!(file = %path.display(), table, reason, "database not read");
            Vec::new()
        }
    }
}

struct Db<'a> {
    bytes: &'a [u8],
    page_size: usize,
    usable: usize,
}

fn rows(bytes: &[u8], table: &str) -> Result<Vec<Row>, &'static str> {
    if bytes.len() < 100 || &bytes[..16] != b"SQLite format 3\0" {
        return Err("not an SQLite database");
    }
    let page_size = match u16::from_be_bytes([bytes[16], bytes[17]]) {
        1 => 65_536,
        n if n >= 512 && n.is_power_of_two() => usize::from(n),
        _ => return Err("page size"),
    };
    // Text encoding 1 is UTF-8; 0 means "not set yet" on an empty database.
    let encoding = u32::from_be_bytes([bytes[56], bytes[57], bytes[58], bytes[59]]);
    if encoding > 1 {
        return Err("text is not UTF-8");
    }
    let usable = page_size - usize::from(bytes[20]);
    if usable < 480 {
        return Err("usable page size");
    }
    let db = Db {
        bytes,
        page_size,
        usable,
    };
    let mut schema = Vec::new();
    db.walk(1, &mut HashSet::new(), 0, &mut schema)?;
    let (root, sql) = schema
        .iter()
        .find_map(|(_, record)| {
            let kind = record.first()?.text()?;
            let name = record.get(1)?.text()?;
            let root = record.get(3)?.int()?;
            let sql = record.get(4)?.text()?;
            (kind == "table" && name.eq_ignore_ascii_case(table)).then(|| (root, sql.to_owned()))
        })
        .ok_or("no such table")?;
    let columns = columns(&sql);
    let alias = rowid_alias(&sql);
    let root = u32::try_from(root).map_err(|_| "root page")?;
    let mut records = Vec::new();
    db.walk(root, &mut HashSet::new(), 0, &mut records)?;
    Ok(records
        .into_iter()
        .map(|(rowid, record)| {
            columns
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    let value = record.get(i).cloned().unwrap_or(Value::Null);
                    // An INTEGER PRIMARY KEY is the rowid, stored as NULL.
                    let value = if value == Value::Null && alias.as_deref() == Some(name.as_str()) {
                        Value::Int(rowid)
                    } else {
                        value
                    };
                    (name.clone(), value)
                })
                .collect()
        })
        .collect())
}

impl Db<'_> {
    fn page(&self, number: u32) -> Result<&[u8], &'static str> {
        let index = usize::try_from(number).map_err(|_| "page number")?;
        let start = index
            .checked_sub(1)
            .and_then(|i| i.checked_mul(self.page_size))
            .ok_or("page number")?;
        self.bytes
            .get(start..start + self.page_size)
            .ok_or("page past the end")
    }

    /// Every `(rowid, record)` of the table b-tree rooted at `number`.
    fn walk(
        &self,
        number: u32,
        seen: &mut HashSet<u32>,
        depth: u32,
        out: &mut Vec<(i64, Vec<Value>)>,
    ) -> Result<(), &'static str> {
        if depth > 32 || !seen.insert(number) {
            return Err("b-tree loop");
        }
        let page = self.page(number)?;
        let header = if number == 1 { 100 } else { 0 };
        let kind = *page.get(header).ok_or("page header")?;
        let cells = usize::from(be16(page, header + 3)?);
        match kind {
            // Interior table page: child pointers, then the right-most one.
            0x05 => {
                for i in 0..cells {
                    let at = usize::from(be16(page, header + 12 + 2 * i)?);
                    self.walk(be32(page, at)?, seen, depth + 1, out)?;
                }
                self.walk(be32(page, header + 8)?, seen, depth + 1, out)
            }
            // Leaf table page: payload size, rowid, payload.
            0x0D => {
                for i in 0..cells {
                    let mut at = usize::from(be16(page, header + 8 + 2 * i)?);
                    let size = varint(page, &mut at)?;
                    // Two's complement, as SQLite stores it.
                    #[allow(clippy::cast_possible_wrap)]
                    let rowid = varint(page, &mut at)? as i64;
                    let payload =
                        self.payload(page, at, usize::try_from(size).map_err(|_| "payload size")?)?;
                    out.push((rowid, record(&payload)?));
                }
                Ok(())
            }
            _ => Err("not a table b-tree page"),
        }
    }

    /// A cell's payload, gathered from its overflow pages when it spills.
    fn payload(&self, page: &[u8], at: usize, size: usize) -> Result<Vec<u8>, &'static str> {
        let u = self.usable;
        let max_local = u - 35;
        let local = if size <= max_local {
            size
        } else {
            let min_local = (u - 12) * 32 / 255 - 23;
            let k = min_local + (size - min_local) % (u - 4);
            if k <= max_local { k } else { min_local }
        };
        let mut out = page
            .get(at..at + local)
            .ok_or("cell past the page")?
            .to_vec();
        if local == size {
            return Ok(out);
        }
        let mut next = be32(page, at + local)?;
        let mut seen = HashSet::new();
        while out.len() < size {
            if next == 0 || !seen.insert(next) {
                return Err("overflow chain");
            }
            let overflow = self.page(next)?;
            next = be32(overflow, 0)?;
            let take = (size - out.len()).min(u - 4);
            out.extend_from_slice(overflow.get(4..4 + take).ok_or("overflow page")?);
        }
        Ok(out)
    }
}

fn be16(page: &[u8], at: usize) -> Result<u16, &'static str> {
    let b = page.get(at..at + 2).ok_or("short read")?;
    Ok(u16::from_be_bytes([b[0], b[1]]))
}

fn be32(page: &[u8], at: usize) -> Result<u32, &'static str> {
    let b = page.get(at..at + 4).ok_or("short read")?;
    Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// `SQLite`'s big-endian variable-length integer: up to 9 bytes.
fn varint(bytes: &[u8], at: &mut usize) -> Result<u64, &'static str> {
    let mut value = 0u64;
    for i in 0..9 {
        let b = *bytes.get(*at).ok_or("short varint")?;
        *at += 1;
        if i == 8 {
            return Ok((value << 8) | u64::from(b));
        }
        value = (value << 7) | u64::from(b & 0x7F);
        if b & 0x80 == 0 {
            return Ok(value);
        }
    }
    Ok(value)
}

/// The values of a record: a header of serial types, then the body.
fn record(payload: &[u8]) -> Result<Vec<Value>, &'static str> {
    let mut at = 0;
    let header_len = usize::try_from(varint(payload, &mut at)?).map_err(|_| "header size")?;
    let mut types = Vec::new();
    while at < header_len {
        types.push(varint(payload, &mut at)?);
    }
    let mut body = header_len;
    let mut values = Vec::with_capacity(types.len());
    for serial in types {
        let (value, len) = match serial {
            0 => (Value::Null, 0),
            1..=6 => {
                let len = [0, 1, 2, 3, 4, 6, 8][usize::try_from(serial).unwrap_or(0)];
                let raw = payload.get(body..body + len).ok_or("short integer")?;
                // Sign-extend from the stored width.
                let mut n = if raw[0] & 0x80 == 0 { 0i64 } else { -1i64 };
                for b in raw {
                    n = (n << 8) | i64::from(*b);
                }
                (Value::Int(n), len)
            }
            7 => {
                let raw = payload.get(body..body + 8).ok_or("short real")?;
                let mut a = [0u8; 8];
                a.copy_from_slice(raw);
                (Value::Real(f64::from_be_bytes(a)), 8)
            }
            8 => (Value::Int(0), 0),
            9 => (Value::Int(1), 0),
            n if n >= 12 && n % 2 == 0 => (
                Value::Blob,
                usize::try_from((n - 12) / 2).map_err(|_| "blob size")?,
            ),
            n if n >= 13 => {
                let len = usize::try_from((n - 13) / 2).map_err(|_| "text size")?;
                let raw = payload.get(body..body + len).ok_or("short text")?;
                (Value::Text(String::from_utf8_lossy(raw).into_owned()), len)
            }
            _ => return Err("reserved serial type"),
        };
        body += len;
        values.push(value);
    }
    Ok(values)
}

/// The column names of a `CREATE TABLE` statement, in order.
fn columns(sql: &str) -> Vec<String> {
    definitions(sql)
        .into_iter()
        .filter_map(|def| {
            let (name, quoted, _) = identifier(def)?;
            let upper = name.to_ascii_uppercase();
            if !quoted
                && ["CONSTRAINT", "PRIMARY", "UNIQUE", "CHECK", "FOREIGN"].contains(&upper.as_str())
            {
                return None;
            }
            Some(name.to_owned())
        })
        .collect()
}

/// The identifier a column definition starts with, whether it was quoted
/// (`"a b"`, `` `a` ``, `[a]`), and the rest of the definition.
fn identifier(def: &str) -> Option<(&str, bool, &str)> {
    let close = match def.chars().next()? {
        '"' => '"',
        '`' => '`',
        '[' => ']',
        _ => {
            let end = def.find(char::is_whitespace).unwrap_or(def.len());
            return Some((&def[..end], false, &def[end..]));
        }
    };
    let rest = &def[1..];
    rest.find(close)
        .map(|end| (&rest[..end], true, &rest[end + 1..]))
}

/// The column declared `INTEGER PRIMARY KEY`, which stores the rowid.
fn rowid_alias(sql: &str) -> Option<String> {
    definitions(sql).into_iter().find_map(|def| {
        let (name, _, rest) = identifier(def)?;
        let words: Vec<String> = rest
            .split_whitespace()
            .map(str::to_ascii_uppercase)
            .collect();
        words
            .starts_with(&["INTEGER".to_owned(), "PRIMARY".to_owned(), "KEY".to_owned()])
            .then(|| name.to_owned())
    })
}

/// The comma-separated definitions between the outer parentheses.
fn definitions(sql: &str) -> Vec<&str> {
    let (Some(open), Some(close)) = (sql.find('('), sql.rfind(')')) else {
        return Vec::new();
    };
    if close <= open {
        return Vec::new();
    }
    let inner = &sql[open + 1..close];
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, c) in inner.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(inner[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(inner[start..].trim());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lutris's own schema for its games table.
    const GAMES: &str = "CREATE TABLE games (id INTEGER PRIMARY KEY, name TEXT, sortname TEXT, slug TEXT, installer_slug TEXT, parent_slug TEXT, platform TEXT, runner TEXT, executable TEXT, directory TEXT, updated DATETIME, lastplayed INTEGER, installed INTEGER, installed_at INTEGER, year INTEGER, configpath TEXT, has_custom_banner INTEGER, has_custom_icon INTEGER, has_custom_coverart_big INTEGER, playtime REAL, service TEXT, service_id TEXT, discord_id TEXT)";

    #[test]
    fn a_create_statement_gives_the_columns_and_the_rowid_alias() {
        let cols = columns(GAMES);
        assert_eq!(cols.len(), 23);
        assert_eq!(cols[1], "name");
        assert_eq!(cols[15], "configpath");
        assert_eq!(rowid_alias(GAMES).as_deref(), Some("id"));
        assert_eq!(
            columns("CREATE TABLE t (\"a b\" TEXT, c INT, PRIMARY KEY (c, \"a b\"))"),
            ["a b", "c"]
        );
        assert_eq!(
            rowid_alias("CREATE TABLE t (\"my id\" integer primary key, x)").as_deref(),
            Some("my id")
        );
    }

    #[test]
    fn varints_and_records_decode() {
        let mut at = 0;
        assert_eq!(varint(&[0x81, 0x00], &mut at), Ok(128));
        // Header: size 4, NULL, int8, text of 2 bytes; body: -2, "hi".
        let rec = [4u8, 0, 1, 17, 0xFE, b'h', b'i'];
        assert_eq!(
            record(&rec),
            Ok(vec![Value::Null, Value::Int(-2), Value::Text("hi".into())])
        );
    }

    #[test]
    fn garbage_is_not_a_database() {
        assert!(rows(b"not a database at all", "games").is_err());
        assert!(rows(&[0u8; 4096], "games").is_err());
    }
}
