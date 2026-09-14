// SPDX-License-Identifier: GPL-3.0-only

use std::{error::Error, fmt};

use crate::domain::{Backend, Container};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    line: usize,
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid container output on line {}", self.line)
    }
}

impl Error for ParseError {}

pub fn parse_container_list(backend: Backend, output: &str) -> Result<Vec<Container>, ParseError> {
    output
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            let fields: Vec<_> = line.splitn(5, '\t').collect();
            if fields.len() != 5 || fields.iter().any(|field| field.is_empty()) {
                return Err(ParseError { line: index + 1 });
            }
            Ok(Container {
                backend,
                id: fields[0].to_owned(),
                image: fields[1].to_owned(),
                name: fields[2].to_owned(),
                state: fields[3].to_owned(),
                status: fields[4].to_owned(),
            })
        })
        .collect()
}
