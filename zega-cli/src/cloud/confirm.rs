//! The question before a delete: a person types the resource's id, or a
//! script says `--yes`. Nothing is sent to the API until it is answered.

use super::api::CloudError;
use std::io::{self, BufRead, IsTerminal, Write};

/// Refuse, before any request, when nobody can be asked: stdin is a pipe, a
/// file or closed, and `--yes` was not given.
pub fn require_terminal(noun: &str) -> Result<(), CloudError> {
    if io::stdin().is_terminal() {
        return Ok(());
    }
    Err(CloudError::Local(format!(
        "refusing to delete the {noun} without asking: stdin is not a terminal. Nothing was deleted. Pass --yes to delete without asking."
    )))
}

/// Ask for the id on the terminal. Anything else, or no answer, cancels.
pub fn ask(id: &str) -> Result<(), CloudError> {
    eprint!("Type {id} to delete it, anything else cancels: ");
    let _ = io::stderr().flush();
    let mut answer = String::new();
    io::stdin()
        .lock()
        .read_line(&mut answer)
        .map_err(|error| format!("cannot read from the terminal: {error}"))?;
    check(&answer, id)
}

fn check(answer: &str, id: &str) -> Result<(), CloudError> {
    if answer.trim() == id {
        Ok(())
    } else {
        Err(CloudError::Local(
            "not confirmed: nothing was deleted".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same question for a reader that is not the process's stdin.
    fn ask_from(reader: &mut impl BufRead, id: &str) -> Result<(), CloudError> {
        let mut answer = String::new();
        reader.read_line(&mut answer).unwrap();
        check(&answer, id)
    }

    #[test]
    fn only_the_exact_id_confirms() {
        assert!(ask_from(&mut "p1\n".as_bytes(), "p1").is_ok());
        assert!(ask_from(&mut "  p1 \r\n".as_bytes(), "p1").is_ok());
        for wrong in ["", "\n", "y\n", "yes\n", "P1\n", "p\n", "p1 p2\n", "p10\n"] {
            assert!(
                ask_from(&mut wrong.as_bytes(), "p1").is_err(),
                "{wrong:?} must not confirm"
            );
        }
    }
}
