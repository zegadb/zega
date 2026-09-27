//! APS 24: sparse valid-time histories, separate from the live node records.
use crate::{
    graph::{NodeId, NodeRef, NodeView},
    Value,
};
use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    OnceLock,
};

pub(crate) type Histories = BTreeMap<(NodeId, String), History>;
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct History {
    pub changes: Vec<(i64, Value)>,
    pub min: Value,
    pub max: Value,
    pub first: i64,
    pub last: i64,
    pub ordered: bool,
}
impl History {
    pub fn new(at: i64, value: Value) -> Self {
        let ordered = compare(&value, &value).is_some();
        Self {
            changes: vec![(at, value.clone())],
            min: value.clone(),
            max: value,
            first: at,
            last: at,
            ordered,
        }
    }
    pub fn insert(&mut self, at: i64, value: Value) {
        match self.changes.binary_search_by_key(&at, |c| c.0) {
            Ok(i) => self.changes[i].1 = value,
            Err(i) => self.changes.insert(i, (at, value)),
        }
        self.first = self.changes[0].0;
        self.last = self.changes[self.changes.len() - 1].0;
        self.min = self.changes[0].1.clone();
        self.max = self.min.clone();
        self.ordered = true;
        for (_, value) in &self.changes {
            self.ordered &= compare(value, &self.min).is_some();
            if compare(value, &self.min) == Some(std::cmp::Ordering::Less) {
                self.min = value.clone();
            }
            if compare(value, &self.max) == Some(std::cmp::Ordering::Greater) {
                self.max = value.clone();
            }
        }
    }
    pub fn at(&self, at: i64) -> Option<&Value> {
        let end = self.changes.partition_point(|c| c.0 <= at);
        end.checked_sub(1).map(|i| &self.changes[i].1)
    }
}
fn compare(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (Value::Int(a), Value::Int(b)) => a.partial_cmp(b),
        (Value::Float(a), Value::Float(b)) => f64::from_bits(*a).partial_cmp(&f64::from_bits(*b)),
        (Value::String(a), Value::String(b)) => a.partial_cmp(b),
        (Value::Bool(a), Value::Bool(b)) => a.partial_cmp(b),
        _ => None,
    }
}
#[derive(Default)]
pub(crate) struct Store {
    encoded: Option<Vec<u8>>,
    decoded: OnceLock<Result<Histories, String>>,
    accesses: AtomicU64,
}
impl Store {
    pub fn has_data(&self) -> bool {
        self.encoded.is_some()
            || self
                .decoded
                .get()
                .is_some_and(|r| r.as_ref().is_ok_and(|h| !h.is_empty()))
    }
    pub fn lazy(bytes: Vec<u8>) -> Self {
        Self {
            encoded: Some(bytes),
            ..Self::default()
        }
    }
    pub fn get(&self) -> Result<&Histories, String> {
        self.accesses.fetch_add(1, Ordering::Relaxed);
        self.decoded
            .get_or_init(|| {
                self.encoded
                    .as_deref()
                    .map(decode)
                    .unwrap_or_else(|| Ok(BTreeMap::new()))
            })
            .as_ref()
            .map_err(Clone::clone)
    }
    pub fn get_mut(&mut self) -> Result<&mut Histories, String> {
        self.get()?;
        self.encoded = None;
        self.decoded
            .get_mut()
            .expect("initialized")
            .as_mut()
            .map_err(|e| e.clone())
    }
    #[cfg(test)]
    pub fn accesses(&self) -> u64 {
        self.accesses.load(Ordering::Relaxed)
    }
    pub fn bytes(&self) -> Result<Option<Vec<u8>>, String> {
        if let Some(bytes) = &self.encoded {
            return Ok(Some(bytes.clone()));
        }
        match self.decoded.get() {
            None => Ok(None),
            Some(Ok(h)) if h.is_empty() => Ok(None),
            Some(Ok(h)) => encode(h).map(Some),
            Some(Err(e)) => Err(e.clone()),
        }
    }
}
/// A borrowed time view; the ordinary NodeRef path never calls this.
#[derive(Clone, Copy)]
pub(crate) struct AsOf<'a> {
    pub node: NodeRef<'a>,
    pub histories: &'a Histories,
    pub at: i64,
}
impl NodeView for AsOf<'_> {
    fn id(&self) -> NodeId {
        self.node.id
    }
    fn has_label(&self, label: &str) -> bool {
        self.node.has_label(label)
    }
    fn first_label(&self) -> Option<&str> {
        self.node.first_label()
    }
    fn prop(&self, key: &str) -> Option<&Value> {
        match self.histories.get(&(self.node.id, key.to_owned())) {
            Some(history) => history.at(self.at),
            None => self.node.prop(key),
        }
    }
}

pub(crate) fn now() -> i64 {
    #[cfg(target_arch = "wasm32")]
    {
        (js_sys::Date::now() / 1000.0) as i64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
    }
}
/// Strict UTC civil dates. The inverse pair uses the Gregorian 400-year cycle.
pub(crate) fn date(s: &str) -> Result<i64, String> {
    let bad = || "expected YYYY-MM-DD or YYYY-MM-DDTHH:MM[:SS] (UTC)".to_string();
    let b = s.as_bytes();
    if !matches!(b.len(), 10 | 16 | 19)
        || b[4] != b'-'
        || b[7] != b'-'
        || (b.len() >= 16 && (b[10] != b'T' || b[13] != b':'))
        || (b.len() == 19 && b[16] != b':')
    {
        return Err(bad());
    }
    if !b
        .iter()
        .enumerate()
        .all(|(i, c)| matches!(i, 4 | 7 | 10 | 13 | 16) || c.is_ascii_digit())
    {
        return Err(bad());
    }
    let y: i64 = s[..4].parse().map_err(|_| bad())?;
    let m: i64 = s[5..7].parse().map_err(|_| bad())?;
    let d: i64 = s[8..10].parse().map_err(|_| bad())?;
    let (h, n) = if b.len() >= 16 {
        (
            s[11..13].parse::<i64>().map_err(|_| bad())?,
            s[14..16].parse::<i64>().map_err(|_| bad())?,
        )
    } else {
        (0, 0)
    };
    let second = if b.len() == 19 {
        s[17..19].parse::<i64>().map_err(|_| bad())?
    } else {
        0
    };
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days = match m {
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => return Err(bad()),
    };
    if d < 1 || d > days || h > 23 || n > 59 || second > 59 {
        return Err(bad());
    }
    let year = y - i64::from(m <= 2);
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let mp = m + if m > 2 { -3 } else { 9 };
    Ok(
        (era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + (153 * mp + 2) / 5 + d - 1 - 719468)
            * 86400
            + h * 3600
            + n * 60
            + second,
    )
}
pub(crate) fn format_date(t: i64) -> String {
    let z = t.div_euclid(86400) + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    y += i64::from(m <= 2);
    let seconds = t.rem_euclid(86400);
    let minute = format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}",
        seconds / 3600,
        (seconds % 3600) / 60
    );
    if seconds % 60 == 0 {
        minute
    } else {
        format!("{minute}:{:02}", seconds % 60)
    }
}
pub(crate) fn inner_type(ty: &str) -> Option<&str> {
    ty.strip_prefix('<')?.strip_suffix('>')
}
pub(crate) fn is_temporal(ty: &str) -> bool {
    ty.starts_with('<')
}
pub(crate) fn plain_type(ty: &str) -> String {
    if let Some(inner) = inner_type(ty) {
        inner.to_string()
    } else if let Some(inner) = ty.strip_prefix('<').and_then(|s| s.strip_suffix(">[]")) {
        format!("{inner}[]")
    } else {
        ty.to_string()
    }
}
fn put(out: &mut Vec<u8>, mut n: u64) {
    while n >= 128 {
        out.push(n as u8 | 128);
        n >>= 7;
    }
    out.push(n as u8);
}
fn zig(n: i64) -> u64 {
    ((n << 1) ^ (n >> 63)) as u64
}
fn unzig(n: u64) -> i64 {
    ((n >> 1) as i64) ^ -((n & 1) as i64)
}
fn blob(out: &mut Vec<u8>, b: &[u8]) {
    put(out, b.len() as u64);
    out.extend_from_slice(b);
}
struct Input<'a>(&'a [u8]);
impl<'a> Input<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if n > self.0.len() {
            return Err("truncated HIST".into());
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn var(&mut self) -> Result<u64, String> {
        let mut n = 0;
        for shift in (0..70).step_by(7) {
            let b = self.take(1)?[0];
            if shift == 63 && b > 1 {
                return Err("HIST varint overflow".into());
            }
            n |= u64::from(b & 127) << shift;
            if b < 128 {
                return Ok(n);
            }
        }
        Err("invalid HIST varint".into())
    }
    fn len(&mut self) -> Result<usize, String> {
        let n = usize::try_from(self.var()?).map_err(|_| "HIST length overflow")?;
        if n > self.0.len() {
            return Err("HIST length exceeds input".into());
        }
        Ok(n)
    }
    fn blob(&mut self) -> Result<&'a [u8], String> {
        let n = self.len()?;
        self.take(n)
    }
}
/// Columns per history: node delta, field dictionary id, time deltas, values.
fn encode(h: &Histories) -> Result<Vec<u8>, String> {
    let mut out = vec![1];
    let mut dict = BTreeMap::<String, u64>::new();
    for ((_, f), history) in h {
        dict.entry(f.clone()).or_default();
        for (_, v) in &history.changes {
            if let Value::String(s) = v {
                dict.entry(s.to_string()).or_default();
            }
        }
    }
    put(&mut out, dict.len() as u64);
    for (i, (s, id)) in dict.iter_mut().enumerate() {
        *id = i as u64;
        blob(&mut out, s.as_bytes());
    }
    put(&mut out, h.len() as u64);
    let mut node = 0;
    for ((id, f), history) in h {
        put(&mut out, id - node);
        node = *id;
        put(&mut out, dict[f]);
        put(&mut out, history.changes.len() as u64);
        let (mut t, mut delta) = (0i64, 0i64);
        for (at, _) in &history.changes {
            let d = at.wrapping_sub(t);
            put(&mut out, zig(d.wrapping_sub(delta)));
            t = *at;
            delta = d;
        }
        let (mut integer, mut float) = (0i64, 0u64);
        for (_, v) in &history.changes {
            match v {
                Value::Int(n) => {
                    out.push(0);
                    put(&mut out, zig(n.wrapping_sub(integer)));
                    integer = *n;
                }
                Value::Float(n) => {
                    out.push(1);
                    put(&mut out, n ^ float);
                    float = *n;
                }
                Value::String(s) => {
                    out.push(2);
                    put(&mut out, dict[s.as_ref()]);
                }
                Value::Null => out.push(3),
                Value::Bool(false) => out.push(4),
                Value::Bool(true) => out.push(5),
                other => {
                    out.push(6);
                    blob(
                        &mut out,
                        &bincode::serialize(other).map_err(|e| e.to_string())?,
                    );
                }
            }
        }
    }
    Ok(out)
}
fn decode(bytes: &[u8]) -> Result<Histories, String> {
    let mut input = Input(bytes);
    if input.take(1)? != [1] {
        return Err("unsupported HIST version".into());
    }
    let n = input.len()?;
    let mut dict = Vec::new();
    for _ in 0..n {
        dict.push(
            std::str::from_utf8(input.blob()?)
                .map_err(|e| e.to_string())?
                .to_string(),
        );
    }
    let n = input.len()?;
    let mut out = BTreeMap::new();
    let mut node = 0u64;
    for _ in 0..n {
        node = node.checked_add(input.var()?).ok_or("HIST node overflow")?;
        let field = dict
            .get(input.var()? as usize)
            .ok_or("HIST dictionary index")?
            .clone();
        let count = input.len()?;
        if count == 0 {
            return Err("empty HIST history".into());
        }
        let (mut t, mut delta) = (0i64, 0i64);
        let mut times = Vec::new();
        for i in 0..count {
            delta = delta.wrapping_add(unzig(input.var()?));
            let next = t.wrapping_add(delta);
            if i > 0 && next <= t {
                return Err("unsorted HIST times".into());
            }
            t = next;
            times.push(t);
        }
        let (mut integer, mut float) = (0i64, 0u64);
        let mut history = None;
        for at in times {
            let value = match input.take(1)?[0] {
                0 => {
                    integer = integer.wrapping_add(unzig(input.var()?));
                    Value::Int(integer)
                }
                1 => {
                    float ^= input.var()?;
                    Value::Float(float)
                }
                2 => Value::String(
                    dict.get(input.var()? as usize)
                        .ok_or("HIST dictionary index")?
                        .clone()
                        .into(),
                ),
                3 => Value::Null,
                4 => Value::Bool(false),
                5 => Value::Bool(true),
                6 => crate::wal::decode_exact(input.blob()?).map_err(|e| e.to_string())?,
                _ => return Err("invalid HIST value tag".into()),
            };
            match &mut history {
                None => history = Some(History::new(at, value)),
                Some(h) => h.insert(at, value),
            }
        }
        if out
            .insert((node, field), history.expect("nonempty"))
            .is_some()
        {
            return Err("duplicate HIST key".into());
        }
    }
    if !input.0.is_empty() {
        return Err("trailing HIST bytes".into());
    }
    Ok(out)
}
