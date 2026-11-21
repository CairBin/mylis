//! 最小 SemVer：解析 `1.2.3`、比较、匹配 `* / ^ / ~ / =` 约束。
//!
//! 只覆盖包解析必需的子集；预发布版本按 SemVer 规则低于同号正式版。

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: Vec<String>,
}

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim().trim_start_matches('v');
        let (core, pre) = match s.split_once('-') {
            Some((a, b)) => (a, b),
            None => (s, ""),
        };
        let mut parts = core.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next().unwrap_or("0").parse().ok()?;
        let patch = parts.next().unwrap_or("0").parse().ok()?;
        if parts.next().is_some() {
            return None;
        }
        let pre = pre
            .split('.')
            .filter(|x| !x.is_empty())
            .map(|x| x.to_string())
            .collect();
        Some(Version { major, minor, patch, pre })
    }

    fn cmp_pre(&self, other: &Version) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        match (self.pre.is_empty(), other.pre.is_empty()) {
            (true, true) => Ordering::Equal,
            (false, true) => Ordering::Less,
            (true, false) => Ordering::Greater,
            (false, false) => {
                for (a, b) in self.pre.iter().zip(other.pre.iter()) {
                    let an = a.parse::<u64>();
                    let bn = b.parse::<u64>();
                    let ord = match (an, bn) {
                        (Ok(x), Ok(y)) => x.cmp(&y),
                        (Ok(_), Err(_)) => Ordering::Less,
                        (Err(_), Ok(_)) => Ordering::Greater,
                        (Err(_), Err(_)) => a.cmp(b),
                    };
                    if ord != Ordering::Equal {
                        return ord;
                    }
                }
                self.pre.len().cmp(&other.pre.len())
            }
        }
    }

    pub fn cmp(&self, other: &Version) -> std::cmp::Ordering {
        self.major
            .cmp(&other.major)
            .then(self.minor.cmp(&other.minor))
            .then(self.patch.cmp(&other.patch))
            .then(self.cmp_pre(other))
    }
}

#[derive(Clone, Debug)]
pub enum Req {
    Any,
    Exact(Version),
    Caret(Version),
    Tilde(Version),
}

impl Req {
    pub fn parse(s: &str) -> Option<Req> {
        let s = s.trim();
        if s.is_empty() || s == "*" {
            return Some(Req::Any);
        }
        if let Some(x) = s.strip_prefix('^') {
            return Some(Req::Caret(Version::parse(x)?));
        }
        if let Some(x) = s.strip_prefix('~') {
            return Some(Req::Tilde(Version::parse(x)?));
        }
        if let Some(x) = s.strip_prefix('=') {
            return Some(Req::Exact(Version::parse(x)?));
        }
        Some(Req::Exact(Version::parse(s)?))
    }

    pub fn matches(&self, v: &Version) -> bool {
        match self {
            Req::Any => true,
            Req::Exact(x) => v == x,
            Req::Caret(x) => {
                if v < x {
                    return false;
                }
                if x.major > 0 {
                    v.major == x.major
                } else if x.minor > 0 {
                    v.major == 0 && v.minor == x.minor
                } else {
                    v == x
                }
            }
            Req::Tilde(x) => v >= x && v.major == x.major && v.minor == x.minor,
        }
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caret_rules() {
        assert!(Req::parse("^0.1.0").unwrap().matches(&Version::parse("0.1.5").unwrap()));
        assert!(!Req::parse("^0.1.0").unwrap().matches(&Version::parse("0.2.0").unwrap()));
        assert!(Req::parse("^1.2.0").unwrap().matches(&Version::parse("1.9.0").unwrap()));
        assert!(!Req::parse("^1.2.0").unwrap().matches(&Version::parse("2.0.0").unwrap()));
        assert!(Req::parse("~1.2.0").unwrap().matches(&Version::parse("1.2.9").unwrap()));
        assert!(!Req::parse("~1.2.0").unwrap().matches(&Version::parse("1.3.0").unwrap()));
    }
}
