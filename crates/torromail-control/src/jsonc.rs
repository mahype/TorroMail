//! Surgical JSONC editing for client-owned configuration files.
//!
//! The editor changes one object path while preserving comments, trailing
//! commas, formatting, and every unrelated byte. It intentionally implements
//! only the object traversal TorroMail needs; inserted values are serialized
//! JSON supplied by the caller.

use std::ops::Range;

#[derive(Debug, Clone, Copy)]
pub(crate) struct Error;

#[derive(Debug, Clone)]
struct Property {
    name: String,
    key_start: usize,
    value_start: usize,
    value_end: usize,
    comma_before: Option<usize>,
    comma_after: Option<usize>,
}

pub(crate) struct Document {
    bytes: Vec<u8>,
}

impl Document {
    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub(crate) fn contains(&self, path: &[&str]) -> Result<bool, Error> {
        let Some(mut object) = self.root_object()? else {
            return Ok(false);
        };
        for (index, component) in path.iter().enumerate() {
            let Some(property) = self
                .properties(object.clone())?
                .into_iter()
                .find(|property| property.name == *component)
            else {
                return Ok(false);
            };
            if index == path.len() - 1 {
                return Ok(true);
            }
            let Some(nested) = self.object_at(property.value_start)? else {
                return Ok(false);
            };
            object = nested;
        }
        Ok(false)
    }

    pub(crate) fn string(&self, path: &[&str]) -> Result<Option<String>, Error> {
        let Some(mut object) = self.root_object()? else {
            return Ok(None);
        };
        for (index, component) in path.iter().enumerate() {
            let Some(property) = self
                .properties(object.clone())?
                .into_iter()
                .find(|property| property.name == *component)
            else {
                return Ok(None);
            };
            if index == path.len() - 1 {
                return self.string_value(property.value_start, property.value_end);
            }
            let Some(nested) = self.object_at(property.value_start)? else {
                return Ok(None);
            };
            object = nested;
        }
        Ok(None)
    }

    pub(crate) fn set(&mut self, path: &[&str], json_value: &str) -> Result<(), Error> {
        if path.is_empty() {
            return Err(Error);
        }
        let Some(root) = self.root_object()? else {
            return Err(Error);
        };
        self.set_in(path, json_value.as_bytes(), root)
    }

    pub(crate) fn remove(&mut self, path: &[&str]) -> Result<(), Error> {
        if path.is_empty() {
            return Err(Error);
        }
        let Some(root) = self.root_object()? else {
            return Err(Error);
        };
        self.remove_in(path, root)
    }

    fn set_in(
        &mut self,
        path: &[&str],
        json_value: &[u8],
        object: Range<usize>,
    ) -> Result<(), Error> {
        let component = path.first().ok_or(Error)?;
        let existing = self
            .properties(object.clone())?
            .into_iter()
            .find(|property| property.name == *component);
        if path.len() == 1 {
            if let Some(existing) = existing {
                self.bytes.splice(
                    existing.value_start..existing.value_end,
                    json_value.iter().copied(),
                );
            } else {
                self.insert_property(component, json_value, object)?;
            }
            return Ok(());
        }

        if let Some(existing) = existing {
            let nested = self.object_at(existing.value_start)?.ok_or(Error)?;
            return self.set_in(&path[1..], json_value, nested);
        }

        let nested = nested_object(&path[1..], json_value);
        self.insert_property(component, &nested, object)
    }

    fn remove_in(&mut self, path: &[&str], object: Range<usize>) -> Result<(), Error> {
        let Some(component) = path.first() else {
            return Ok(());
        };
        let Some(property) = self
            .properties(object)?
            .into_iter()
            .find(|property| property.name == *component)
        else {
            return Ok(());
        };
        if path.len() > 1 {
            let Some(nested) = self.object_at(property.value_start)? else {
                return Ok(());
            };
            return self.remove_in(&path[1..], nested);
        }

        if let Some(comma) = property.comma_after {
            self.bytes.drain(property.key_start..=comma);
        } else if let Some(comma) = property.comma_before {
            self.bytes.drain(comma..property.value_end);
        } else {
            self.bytes.drain(property.key_start..property.value_end);
        }
        Ok(())
    }

    fn root_object(&self) -> Result<Option<Range<usize>>, Error> {
        let mut start = usize::from(self.bytes.starts_with(&[0xEF, 0xBB, 0xBF])) * 3;
        start = self.skip_trivia(start, self.bytes.len());
        if self.bytes.get(start) != Some(&b'{') {
            return Ok(None);
        }
        let root = self.object_at(start)?.ok_or(Error)?;
        if self.skip_trivia(root.end, self.bytes.len()) != self.bytes.len() {
            return Err(Error);
        }
        Ok(Some(root))
    }

    fn object_at(&self, raw_start: usize) -> Result<Option<Range<usize>>, Error> {
        let start = self.skip_trivia(raw_start, self.bytes.len());
        if self.bytes.get(start) != Some(&b'{') {
            return Ok(None);
        }
        let mut depth = 0_usize;
        let mut index = start;
        while index < self.bytes.len() {
            if let Some(next) = self.skipped_string_or_comment(index, self.bytes.len())? {
                index = next;
                continue;
            }
            match self.bytes[index] {
                b'{' => depth += 1,
                b'}' => {
                    depth = depth.checked_sub(1).ok_or(Error)?;
                    if depth == 0 {
                        return Ok(Some(start..index + 1));
                    }
                }
                _ => {}
            }
            index += 1;
        }
        Err(Error)
    }

    fn properties(&self, object: Range<usize>) -> Result<Vec<Property>, Error> {
        let closing_brace = object.end.checked_sub(1).ok_or(Error)?;
        let mut index = self.skip_trivia(object.start + 1, closing_brace);
        let mut previous_comma = None;
        let mut result = Vec::new();

        while index < closing_brace {
            let key_start = index;
            let (name, key_end) = self.parsed_key(index, closing_brace)?.ok_or(Error)?;
            index = self.skip_trivia(key_end, closing_brace);
            if self.bytes.get(index) != Some(&b':') {
                return Err(Error);
            }
            let value_start = self.skip_trivia(index + 1, closing_brace);
            if value_start >= closing_brace {
                return Err(Error);
            }
            let separator = self.value_separator(value_start, closing_brace)?;
            let comma =
                (separator < closing_brace && self.bytes[separator] == b',').then_some(separator);
            result.push(Property {
                name,
                key_start,
                value_start,
                value_end: separator,
                comma_before: previous_comma,
                comma_after: comma,
            });
            let Some(comma) = comma else {
                break;
            };
            previous_comma = Some(comma);
            index = self.skip_trivia(comma + 1, closing_brace);
            if index == closing_brace {
                break;
            }
        }
        Ok(result)
    }

    fn parsed_key(&self, start: usize, limit: usize) -> Result<Option<(String, usize)>, Error> {
        if start >= limit {
            return Ok(None);
        }
        if self.bytes[start] == b'"' || self.bytes[start] == b'\'' {
            let quote = self.bytes[start];
            let mut index = start + 1;
            let mut value = Vec::new();
            while index < limit {
                if self.bytes[index] == b'\\' {
                    if index + 1 >= limit {
                        return Err(Error);
                    }
                    value.push(self.bytes[index + 1]);
                    index += 2;
                } else if self.bytes[index] == quote {
                    return Ok(Some((
                        String::from_utf8(value).map_err(|_| Error)?,
                        index + 1,
                    )));
                } else {
                    value.push(self.bytes[index]);
                    index += 1;
                }
            }
            return Err(Error);
        }

        let mut index = start;
        while index < limit {
            let byte = self.bytes[index];
            if byte == b':' || is_whitespace(byte) {
                break;
            }
            if byte == b'/'
                && index + 1 < limit
                && (self.bytes[index + 1] == b'/' || self.bytes[index + 1] == b'*')
            {
                break;
            }
            index += 1;
        }
        if index == start {
            return Ok(None);
        }
        Ok(Some((
            String::from_utf8(self.bytes[start..index].to_vec()).map_err(|_| Error)?,
            index,
        )))
    }

    fn string_value(&self, start: usize, limit: usize) -> Result<Option<String>, Error> {
        if start >= limit || self.bytes[start] != b'"' {
            return Ok(None);
        }
        let Some(end) = self.skipped_string_or_comment(start, limit)? else {
            return Ok(None);
        };
        if self.skip_trivia(end, limit) != limit {
            return Ok(None);
        }
        Ok(serde_json::from_slice(&self.bytes[start..end]).ok())
    }

    fn value_separator(&self, start: usize, object_end: usize) -> Result<usize, Error> {
        let mut braces = 0_usize;
        let mut brackets = 0_usize;
        let mut index = start;
        while index < object_end {
            if let Some(next) = self.skipped_string_or_comment(index, object_end)? {
                index = next;
                continue;
            }
            match self.bytes[index] {
                b'{' => braces += 1,
                b'}' if braces > 0 => braces -= 1,
                b'[' => brackets += 1,
                b']' if brackets > 0 => brackets -= 1,
                b',' if braces == 0 && brackets == 0 => return Ok(index),
                _ => {}
            }
            index += 1;
        }
        Ok(object_end)
    }

    fn skipped_string_or_comment(
        &self,
        start: usize,
        limit: usize,
    ) -> Result<Option<usize>, Error> {
        if start >= limit {
            return Ok(None);
        }
        if self.bytes[start] == b'"' || self.bytes[start] == b'\'' {
            let quote = self.bytes[start];
            let mut index = start + 1;
            while index < limit {
                if self.bytes[index] == b'\\' {
                    index += 2;
                } else if self.bytes[index] == quote {
                    return Ok(Some(index + 1));
                } else {
                    index += 1;
                }
            }
            return Err(Error);
        }
        if self.bytes[start] != b'/' || start + 1 >= limit {
            return Ok(None);
        }
        if self.bytes[start + 1] == b'/' {
            let mut index = start + 2;
            while index < limit && self.bytes[index] != b'\n' && self.bytes[index] != b'\r' {
                index += 1;
            }
            return Ok(Some(index));
        }
        if self.bytes[start + 1] == b'*' {
            let mut index = start + 2;
            while index + 1 < limit {
                if self.bytes[index] == b'*' && self.bytes[index + 1] == b'/' {
                    return Ok(Some(index + 2));
                }
                index += 1;
            }
            return Err(Error);
        }
        Ok(None)
    }

    fn skip_trivia(&self, start: usize, limit: usize) -> usize {
        let mut index = start;
        while index < limit {
            if is_whitespace(self.bytes[index]) {
                index += 1;
                continue;
            }
            if self.bytes[index] == b'/' && index + 1 < limit && self.bytes[index + 1] == b'/' {
                index += 2;
                while index < limit && self.bytes[index] != b'\n' && self.bytes[index] != b'\r' {
                    index += 1;
                }
                continue;
            }
            if self.bytes[index] == b'/' && index + 1 < limit && self.bytes[index + 1] == b'*' {
                index += 2;
                while index + 1 < limit
                    && !(self.bytes[index] == b'*' && self.bytes[index + 1] == b'/')
                {
                    index += 1;
                }
                if index + 1 < limit {
                    index += 2;
                }
                continue;
            }
            break;
        }
        index
    }

    fn insert_property(
        &mut self,
        name: &str,
        value: &[u8],
        object: Range<usize>,
    ) -> Result<(), Error> {
        let properties = self.properties(object.clone())?;
        let closing_brace = object.end.checked_sub(1).ok_or(Error)?;
        let closing_indent = self.indentation_before(closing_brace);
        let child_indent = format!("{closing_indent}  ");
        let needs_comma = properties
            .last()
            .is_some_and(|property| property.comma_after.is_none());
        let prefix = format!(
            "{}\n{child_indent}\"{name}\": ",
            if needs_comma { "," } else { "" }
        );
        let mut addition = prefix.into_bytes();
        addition.extend_from_slice(value);
        addition.extend_from_slice(format!("\n{closing_indent}").as_bytes());
        self.bytes.splice(closing_brace..closing_brace, addition);
        Ok(())
    }

    fn indentation_before(&self, index: usize) -> String {
        let mut line_start = index;
        while line_start > 0
            && self.bytes[line_start - 1] != b'\n'
            && self.bytes[line_start - 1] != b'\r'
        {
            line_start -= 1;
        }
        let candidate = &self.bytes[line_start..index];
        if candidate.iter().all(|byte| *byte == b' ' || *byte == b'\t') {
            String::from_utf8_lossy(candidate).into_owned()
        } else {
            String::new()
        }
    }
}

fn nested_object(path: &[&str], json_value: &[u8]) -> Vec<u8> {
    let Some(first) = path.first() else {
        return json_value.to_vec();
    };
    let mut result = format!("{{\"{first}\":").into_bytes();
    result.extend_from_slice(&nested_object(&path[1..], json_value));
    result.push(b'}');
    result
}

fn is_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0C)
}
