use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, io, path::Path};

/// Internal port identity. It never occupies an answer slot.
pub(in crate::editor) const OTHERWISE: usize = usize::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(in crate::editor) enum Value {
    Flag(bool),
    Number(i64),
}

impl Value {
    pub fn label(self) -> String {
        match self {
            Self::Flag(value) => if value { "Да" } else { "Нет" }.into(),
            Self::Number(value) => value.to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::editor) enum Operator {
    #[default]
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl Operator {
    pub const ALL: [Self; 6] = [Self::Eq, Self::Ne, Self::Lt, Self::Le, Self::Gt, Self::Ge];
    pub fn label(self) -> &'static str {
        match self {
            Self::Eq => "=",
            Self::Ne => "≠",
            Self::Lt => "<",
            Self::Le => "≤",
            Self::Gt => ">",
            Self::Ge => "≥",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::editor) struct Rule {
    pub variable: u64,
    pub operator: Operator,
    pub value: Value,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::editor) struct Condition {
    #[serde(default)]
    pub any: bool,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

impl Condition {
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    pub fn allows(&self, values: &BTreeMap<u64, Value>) -> bool {
        if self
            .rules
            .iter()
            .any(|rule| match (values.get(&rule.variable), rule.value) {
                (Some(Value::Flag(_)), Value::Flag(_)) => {
                    !matches!(rule.operator, Operator::Eq | Operator::Ne)
                }
                (Some(Value::Number(_)), Value::Number(_)) => false,
                _ => true,
            })
        {
            return false;
        }
        let matches = |rule: &Rule| match (values.get(&rule.variable), rule.value) {
            (Some(Value::Flag(actual)), Value::Flag(expected)) => match rule.operator {
                Operator::Eq => *actual == expected,
                Operator::Ne => *actual != expected,
                _ => false,
            },
            (Some(Value::Number(actual)), Value::Number(expected)) => match rule.operator {
                Operator::Eq => *actual == expected,
                Operator::Ne => *actual != expected,
                Operator::Lt => *actual < expected,
                Operator::Le => *actual <= expected,
                Operator::Gt => *actual > expected,
                Operator::Ge => *actual >= expected,
            },
            _ => false,
        };
        if self.any && !self.is_empty() {
            self.rules.iter().any(matches)
        } else {
            self.rules.iter().all(matches)
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(in crate::editor) struct Variable {
    pub name: String,
    pub default: Value,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(in crate::editor) struct Catalog {
    pub next_id: u64,
    pub variables: BTreeMap<u64, Variable>,
}

impl Catalog {
    fn path(root: &Path) -> std::path::PathBuf {
        root.join(".dialogue-variables.json")
    }

    pub fn read(root: &Path) -> Result<Self, String> {
        match fs::read(Self::path(root)) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| error.to_string()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn save(&self, root: &Path) -> Result<(), String> {
        let path = Self::path(root);
        let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
        let bytes = serde_json::to_vec_pretty(self).map_err(|error| error.to_string())?;
        fs::write(&tmp, bytes).map_err(|error| error.to_string())?;
        fs::rename(&tmp, path).map_err(|error| {
            let _ = fs::remove_file(&tmp);
            error.to_string()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn condition_modes_require_matching_types_and_use_all_integer_comparisons() {
        let values = BTreeMap::from([(1, Value::Flag(true)), (2, Value::Number(7))]);
        let expected = [false, true, false, false, true, true];
        for (operator, expected) in Operator::ALL.into_iter().zip(expected) {
            let condition = Condition {
                any: false,
                rules: vec![Rule {
                    variable: 2,
                    operator,
                    value: Value::Number(5),
                }],
            };
            assert_eq!(condition.allows(&values), expected);
        }
        let mut condition = Condition {
            any: false,
            rules: vec![
                Rule {
                    variable: 1,
                    operator: Operator::Eq,
                    value: Value::Flag(true),
                },
                Rule {
                    variable: 2,
                    operator: Operator::Ne,
                    value: Value::Flag(false),
                },
            ],
        };
        assert!(!condition.allows(&values));
        condition.any = true;
        assert!(!condition.allows(&values));
        condition.rules[1].value = Value::Number(7);
        assert!(condition.allows(&values));
        condition.rules[0].variable = 99;
        assert!(!condition.allows(&values));
        condition.rules.clear();
        assert!(condition.allows(&values));
    }
}
