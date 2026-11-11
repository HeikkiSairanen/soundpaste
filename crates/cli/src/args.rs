//! Command-line argument parsing: `--key value`, `--flag` and positional arguments.

use std::collections::HashMap;

pub struct Args {
    pub positional: Vec<String>,
    flags: HashMap<String, String>,
}

impl Args {
    /// `bool_flags`: flags that take no value.
    pub fn parse(raw: &[String], bool_flags: &[&str]) -> Result<Args, String> {
        let mut positional = Vec::new();
        let mut flags = HashMap::new();
        let mut i = 0;
        while i < raw.len() {
            let a = &raw[i];
            if let Some(name) = a.strip_prefix("--") {
                if bool_flags.contains(&name) {
                    flags.insert(name.to_string(), "1".to_string());
                } else {
                    i += 1;
                    let v = raw.get(i).ok_or_else(|| format!("--{name} needs a value"))?;
                    flags.insert(name.to_string(), v.clone());
                }
            } else if a == "-o" {
                i += 1;
                let v = raw.get(i).ok_or("-o needs a value")?;
                flags.insert("out".to_string(), v.clone());
            } else {
                positional.push(a.clone());
            }
            i += 1;
        }
        Ok(Args { positional, flags })
    }

    pub fn has(&self, name: &str) -> bool {
        self.flags.contains_key(name)
    }
    pub fn str(&self, name: &str) -> Option<&str> {
        self.flags.get(name).map(|s| s.as_str())
    }
    pub fn str_or<'a>(&'a self, name: &str, default: &'a str) -> &'a str {
        self.str(name).unwrap_or(default)
    }
    pub fn num<T: std::str::FromStr>(&self, name: &str, default: T) -> Result<T, String> {
        match self.flags.get(name) {
            None => Ok(default),
            Some(s) => s.parse().map_err(|_| format!("--{name}: invalid value '{s}'")),
        }
    }
}
