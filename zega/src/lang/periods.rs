use super::*;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Period {
    pub from: String,
    pub to: String,
    pub named: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TimeDate {
    Instant(i64),
    Year(i32, bool),
    Period(Box<PeriodName>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PeriodName {
    pub word: String,
    pub name: Json,
    pub end: bool,
}

impl TimeDate {
    pub(crate) fn instant(&self) -> Result<i64> {
        match self {
            Self::Instant(at) => Ok(*at),
            _ => Err(Error::bare("unresolved named date")),
        }
    }
    pub(super) fn end_of_span(&self) -> Option<Self> {
        match self {
            Self::Year(year, _) => Some(Self::Year(*year, true)),
            Self::Period(period) => {
                let mut period = period.clone();
                period.end = true;
                Some(Self::Period(period))
            }
            _ => None,
        }
    }
}

impl Parser<'_> {
    pub(super) fn type_directive(&self, word: &str) -> bool {
        let mut p = self.fork();
        if !p.eat_word(word) {
            return false;
        }
        match word {
            "period" => p.eat_word("from"),
            "calendar" => p.ident().is_ok() && p.eat("->"),
            "appears" | "ends" => p.eat_word("at"),
            _ => false,
        }
    }

    pub(super) fn time_endpoint(&mut self, end: bool) -> Result<TimeDate> {
        self.skip();
        let explicit = if self.eat_word("start") {
            Some(false)
        } else if self.eat_word("end") {
            Some(true)
        } else {
            None
        };
        if explicit.is_some() {
            self.expect_word("of")?;
        }
        self.skip();
        if self
            .src
            .as_bytes()
            .get(self.i)
            .is_some_and(u8::is_ascii_alphabetic)
        {
            let word = self.ident()?.0;
            self.skip();
            let start = self.i;
            let mut look = self.i;
            while self
                .src
                .as_bytes()
                .get(look)
                .is_some_and(|b| b.is_ascii_digit() || *b == b'-')
            {
                look += 1;
            }
            let token = &self.src[start..look];
            let name =
                if token.bytes().filter(|b| *b == b'-').count() == 1 && !token.starts_with('-') {
                    self.i = look;
                    Json::String(token.to_owned())
                } else {
                    self.parse_value()?
                };
            return Ok(TimeDate::Period(Box::new(PeriodName {
                word,
                name,
                end: explicit.unwrap_or(end),
            })));
        }
        if explicit.is_some() {
            return Err(self.err("start of / end of needs a calendar word and period name"));
        }
        let start = self.i;
        let at = self.time_date()?;
        let text = &self.src[start..self.i];
        if text.len() == 4 {
            return Ok(TimeDate::Year(text.parse().expect("parsed year"), end));
        }
        Ok(TimeDate::Instant(at))
    }
}
