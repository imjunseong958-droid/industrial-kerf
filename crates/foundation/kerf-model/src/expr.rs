//! A number worked out from the time: what a parameter is when a document
//! writes it as `(…)`, as in `rotation=(t*90)` or `x=(sin(t*6.28)*0.1)`.
//!
//! `t` is seconds from the clip's first frame. There are numbers, `+ - * /`,
//! parentheses, `pi`, and a few functions: `sin`, `cos`, `abs`, `floor`,
//! `min`, `max`, and `noise`, a smooth wander between -1 and 1 that is the
//! same at the same number on every machine. Nothing else is read, so an
//! expression can do nothing but give a number.

use crate::Time;
use std::fmt;

/// An expression as it was written, and what it means.
#[derive(Clone, Debug, PartialEq)]
pub struct Expression {
    node: Node,
    /// Seconds added to the clip time before `t` is read: what a clip's cut
    /// or trimmed head took away, so the rest plays as it did.
    offset: f64,
    /// The least and the most it may give, as its parameter allows.
    range: (Option<f64>, Option<f64>),
}

#[derive(Clone, Debug, PartialEq)]
enum Node {
    Number(f64),
    Time,
    Neg(Box<Node>),
    Op(char, Box<Node>, Box<Node>),
    Call(Function, Vec<Node>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Function {
    Sin,
    Cos,
    Abs,
    Floor,
    Min,
    Max,
    Noise,
}

impl Function {
    const ALL: [(&'static str, Self, usize); 7] = [
        ("sin", Self::Sin, 1),
        ("cos", Self::Cos, 1),
        ("abs", Self::Abs, 1),
        ("floor", Self::Floor, 1),
        ("min", Self::Min, 2),
        ("max", Self::Max, 2),
        ("noise", Self::Noise, 1),
    ];

    fn name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(_, function, _)| *function == self)
            .map_or("", |(name, ..)| name)
    }
}

/// The deepest an expression may nest, so reading one cannot run away.
const MAX_DEPTH: usize = 32;

impl Expression {
    /// Read what is between the parentheses of `(…)`; spaces are passed
    /// over. What is wrong is said in words.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut reader = Reader {
            text: text.as_bytes(),
            at: 0,
            depth: 0,
        };
        let node = reader.sum()?;
        reader.spaces();
        if reader.at < reader.text.len() {
            return Err(format!(
                "`{}` is not part of an expression",
                &text[reader.at..]
            ));
        }
        Ok(Self {
            node,
            offset: 0.0,
            range: (None, None),
        })
    }

    /// The same, kept between `min` and `max` where its parameter has them.
    pub fn within(mut self, min: Option<f64>, max: Option<f64>) -> Self {
        self.range = (min, max);
        self
    }

    /// Its number at clip time `at`. What is not a number (a division by
    /// zero) is 0.
    pub fn value_at(&self, at: Time) -> f64 {
        let value = eval(&self.node, at.seconds() + self.offset);
        let value = if value.is_finite() { value } else { 0.0 };
        let value = self.range.0.map_or(value, |min| value.max(min));
        self.range.1.map_or(value, |max| value.min(max))
    }

    /// The same expression for a clip that starts `by` later in it, as the
    /// second half of a cut does.
    pub fn starting(&self, by: Time) -> Self {
        Self {
            offset: self.offset + by.seconds(),
            ..self.clone()
        }
    }

    /// It added to (`+`) or multiplied by (`*`) `number`, as a look does to
    /// a clip's own value.
    pub fn with(&self, op: char, number: f64) -> Self {
        Self {
            node: Node::Op(
                op,
                Box::new(self.node.clone()),
                Box::new(Node::Number(number)),
            ),
            ..self.clone()
        }
    }

    /// Whether it gives the same number at every time.
    pub fn is_constant(&self) -> bool {
        !reads_time(&self.node)
    }
}

fn reads_time(node: &Node) -> bool {
    match node {
        Node::Number(_) => false,
        Node::Time => true,
        Node::Neg(inner) => reads_time(inner),
        Node::Op(_, left, right) => reads_time(left) || reads_time(right),
        Node::Call(_, arguments) => arguments.iter().any(reads_time),
    }
}

fn eval(node: &Node, t: f64) -> f64 {
    match node {
        Node::Number(number) => *number,
        Node::Time => t,
        Node::Neg(inner) => -eval(inner, t),
        Node::Op(op, left, right) => {
            let (left, right) = (eval(left, t), eval(right, t));
            match op {
                '+' => left + right,
                '-' => left - right,
                '*' => left * right,
                _ => left / right,
            }
        }
        Node::Call(function, arguments) => {
            let argument = |index: usize| arguments.get(index).map_or(0.0, |node| eval(node, t));
            match function {
                Function::Sin => argument(0).sin(),
                Function::Cos => argument(0).cos(),
                Function::Abs => argument(0).abs(),
                Function::Floor => argument(0).floor(),
                Function::Min => argument(0).min(argument(1)),
                Function::Max => argument(0).max(argument(1)),
                Function::Noise => noise(argument(0)),
            }
        }
    }
}

/// A smooth wander between -1 and 1: a value fixed at each whole number,
/// eased between them.
fn noise(x: f64) -> f64 {
    if !x.is_finite() {
        return 0.0;
    }
    let at = |whole: f64| {
        // A hash of the whole number, the same wherever it runs.
        let mut bits = (whole as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        bits ^= bits >> 32;
        bits = bits.wrapping_mul(0xD6E8_FEB8_6659_FD93);
        bits ^= bits >> 32;
        (bits >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    };
    let whole = x.floor();
    let part = x - whole;
    let eased = part * part * (3.0 - 2.0 * part);
    at(whole) + (at(whole + 1.0) - at(whole)) * eased
}

struct Reader<'a> {
    text: &'a [u8],
    at: usize,
    depth: usize,
}

impl Reader<'_> {
    fn spaces(&mut self) {
        while self.text.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.spaces();
        self.text.get(self.at).copied()
    }

    fn take(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn sum(&mut self) -> Result<Node, String> {
        let mut left = self.product()?;
        while let Some(op @ (b'+' | b'-')) = self.peek() {
            self.at += 1;
            left = Node::Op(char::from(op), Box::new(left), Box::new(self.product()?));
        }
        Ok(left)
    }

    fn product(&mut self) -> Result<Node, String> {
        let mut left = self.unary()?;
        while let Some(op @ (b'*' | b'/')) = self.peek() {
            self.at += 1;
            left = Node::Op(char::from(op), Box::new(left), Box::new(self.unary()?));
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Node, String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err("the expression nests too deep".to_owned());
        }
        let node = if self.take(b'-') {
            Node::Neg(Box::new(self.unary()?))
        } else {
            self.atom()?
        };
        self.depth -= 1;
        Ok(node)
    }

    fn atom(&mut self) -> Result<Node, String> {
        match self.peek() {
            Some(b'(') => {
                self.at += 1;
                let inner = self.sum()?;
                if !self.take(b')') {
                    return Err("a `(` is not closed".to_owned());
                }
                Ok(inner)
            }
            Some(byte) if byte.is_ascii_digit() || byte == b'.' => {
                let start = self.at;
                while self
                    .text
                    .get(self.at)
                    .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'.')
                {
                    self.at += 1;
                }
                let written = std::str::from_utf8(&self.text[start..self.at]).unwrap_or("");
                written
                    .parse()
                    .map(Node::Number)
                    .map_err(|_| format!("`{written}` is not a number"))
            }
            Some(byte) if byte.is_ascii_alphabetic() => {
                let start = self.at;
                while self.text.get(self.at).is_some_and(u8::is_ascii_alphabetic) {
                    self.at += 1;
                }
                let name = std::str::from_utf8(&self.text[start..self.at]).unwrap_or("");
                match name {
                    "t" => return Ok(Node::Time),
                    "pi" => return Ok(Node::Number(std::f64::consts::PI)),
                    _ => {}
                }
                let Some((_, function, count)) =
                    Function::ALL.iter().find(|(known, ..)| *known == name)
                else {
                    let known: Vec<&str> = Function::ALL.iter().map(|(name, ..)| *name).collect();
                    return Err(format!(
                        "`{name}` is not known in an expression: there are `t`, `pi`, and {}",
                        known.join(", ")
                    ));
                };
                if !self.take(b'(') {
                    return Err(format!("`{name}` takes its numbers in parentheses"));
                }
                let mut arguments = vec![self.sum()?];
                while self.take(b',') {
                    arguments.push(self.sum()?);
                }
                if !self.take(b')') {
                    return Err(format!("`{name}(` is not closed"));
                }
                if arguments.len() != *count {
                    return Err(format!(
                        "`{name}` takes {count} number{}, not {}",
                        if *count == 1 { "" } else { "s" },
                        arguments.len()
                    ));
                }
                Ok(Node::Call(*function, arguments))
            }
            Some(byte) => Err(format!(
                "`{}` is not part of an expression",
                char::from(byte)
            )),
            None => Err("the expression ends early".to_owned()),
        }
    }
}

/// How tightly a node binds, to print only the parentheses it needs.
fn binding(node: &Node) -> u8 {
    match node {
        Node::Op('+' | '-', ..) => 1,
        Node::Op(..) => 2,
        Node::Neg(_) => 3,
        _ => 4,
    }
}

fn write_node(node: &Node, out: &mut fmt::Formatter<'_>) -> fmt::Result {
    let child = |inner: &Node, least: u8, out: &mut fmt::Formatter<'_>| {
        if binding(inner) < least {
            write!(out, "(")?;
            write_node(inner, out)?;
            write!(out, ")")
        } else {
            write_node(inner, out)
        }
    };
    match node {
        Node::Number(number) if *number == std::f64::consts::PI => write!(out, "pi"),
        Node::Number(number) => write!(out, "{number}"),
        Node::Time => write!(out, "t"),
        Node::Neg(inner) => {
            write!(out, "-")?;
            child(inner, 3, out)
        }
        Node::Op(op, left, right) => {
            let own = binding(node);
            child(left, own, out)?;
            write!(out, "{op}")?;
            // What is taken away or divided by keeps its own parentheses.
            child(right, own + 1, out)
        }
        Node::Call(function, arguments) => {
            write!(out, "{}(", function.name())?;
            for (index, argument) in arguments.iter().enumerate() {
                if index > 0 {
                    write!(out, ",")?;
                }
                write_node(argument, out)?;
            }
            write!(out, ")")
        }
    }
}

/// As a document writes it between its parentheses, with no spaces. A clip
/// cut from a later part of it reads `t` that much on.
impl fmt::Display for Expression {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.offset == 0.0 {
            return write_node(&self.node, out);
        }
        let moved = substitute(&self.node, self.offset);
        write_node(&moved, out)
    }
}

/// `node` with `t` read `offset` seconds on.
fn substitute(node: &Node, offset: f64) -> Node {
    let again = |inner: &Node| Box::new(substitute(inner, offset));
    match node {
        Node::Time => Node::Op('+', Box::new(Node::Time), Box::new(Node::Number(offset))),
        Node::Number(_) => node.clone(),
        Node::Neg(inner) => Node::Neg(again(inner)),
        Node::Op(op, left, right) => Node::Op(*op, again(left), again(right)),
        Node::Call(function, arguments) => Node::Call(
            *function,
            arguments
                .iter()
                .map(|argument| substitute(argument, offset))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(expression: &str, seconds: f64) -> f64 {
        Expression::parse(expression)
            .unwrap_or_else(|error| panic!("{expression}: {error}"))
            .value_at(Time::from_seconds(seconds).expect("a time"))
    }

    #[test]
    fn it_works_a_number_out_from_the_time() {
        assert_eq!(at("t*90", 2.0), 180.0);
        assert_eq!(at("1+2*3", 0.0), 7.0);
        assert_eq!(at("(1+2)*3", 0.0), 9.0);
        assert_eq!(at("-t+10-2-3", 1.0), 4.0);
        assert_eq!(at("8/2/2", 0.0), 2.0);
        assert_eq!(at(" max( 0 , min(t, 1) ) ", 5.0), 1.0);
        assert_eq!(at("floor(t*24)/24", 0.51), 12.0 / 24.0);
        assert!((at("sin(t*pi)", 0.5) - 1.0).abs() < 1e-12);
        assert!((at("cos(0)+abs(-2)", 0.0) - 3.0).abs() < 1e-12);
        // What is not a number is 0.
        assert_eq!(at("1/(t-t)", 3.0), 0.0);
    }

    #[test]
    fn noise_wanders_smoothly_and_the_same_everywhere() {
        let values: Vec<f64> = (0..200)
            .map(|step| at("noise(t)", f64::from(step) * 0.05))
            .collect();
        assert!(values.iter().all(|value| (-1.0..=1.0).contains(value)));
        assert!(values
            .windows(2)
            .all(|pair| (pair[0] - pair[1]).abs() < 0.2));
        assert!(
            values.iter().any(|value| *value > 0.3) && values.iter().any(|value| *value < -0.3)
        );
        // Pinned: a document draws the same on every machine.
        assert_eq!(at("noise(t)", 0.0), at("noise(0)", 9.0));
        assert!(
            (at("noise(1)", 0.0) + 0.812_636_145_103_548_6).abs() < 1e-12,
            "{}",
            at("noise(1)", 0.0)
        );
    }

    #[test]
    fn it_prints_as_it_reads_with_the_parentheses_it_needs() {
        for (written, printed) in [
            ("t * 90", "t*90"),
            ("(1+2)*3", "(1+2)*3"),
            ("1+(2*3)", "1+2*3"),
            ("10-(2-3)", "10-(2-3)"),
            ("8/(2/2)", "8/(2/2)"),
            ("-(t+1)", "-(t+1)"),
            ("sin( t*pi ) * 0.1", "sin(t*pi)*0.1"),
            ("max(0,min(t,1))", "max(0,min(t,1))"),
        ] {
            let expression = Expression::parse(written).unwrap();
            assert_eq!(expression.to_string(), printed);
            let again = Expression::parse(printed).unwrap();
            assert_eq!(again.to_string(), printed);
            for step in 0..5 {
                let time = Time::from_seconds(f64::from(step) * 0.37).unwrap();
                assert_eq!(expression.value_at(time), again.value_at(time), "{written}");
            }
        }
    }

    #[test]
    fn a_later_start_reads_the_time_that_much_on_and_prints_so() {
        let whole = Expression::parse("sin(t)*t").unwrap();
        let later = whole.starting(Time::from_seconds(1.5).unwrap());
        for step in 0..5 {
            let time = Time::from_seconds(f64::from(step) * 0.25).unwrap();
            let on = Time::from_seconds(f64::from(step) * 0.25 + 1.5).unwrap();
            assert_eq!(later.value_at(time), whole.value_at(on));
        }
        assert_eq!(later.to_string(), "sin(t+1.5)*(t+1.5)");
        let read = Expression::parse(&later.to_string()).unwrap();
        let time = Time::from_seconds(0.5).unwrap();
        assert!((read.value_at(time) - later.value_at(time)).abs() < 1e-12);
    }

    #[test]
    fn it_stays_inside_what_its_parameter_allows() {
        let kept = Expression::parse("t*10")
            .unwrap()
            .within(Some(0.0), Some(5.0));
        assert_eq!(kept.value_at(Time::from_seconds(0.2).unwrap()), 2.0);
        assert_eq!(kept.value_at(Time::from_seconds(3.0).unwrap()), 5.0);
        assert!(Expression::parse("2+3").unwrap().is_constant());
        assert!(!Expression::parse("2+t").unwrap().is_constant());
    }

    #[test]
    fn what_is_wrong_is_said() {
        let wrong = |text: &str| Expression::parse(text).unwrap_err();
        assert!(wrong("t*").contains("ends early"));
        assert!(wrong("(t+1").contains("not closed"));
        assert!(wrong("wiggle(t)").contains("`wiggle` is not known"));
        assert!(wrong("sin t").contains("takes its numbers in parentheses"));
        assert!(wrong("min(t)").contains("takes 2 numbers, not 1"));
        assert!(wrong("t 2").contains("is not part of an expression"));
        assert!(wrong("t$").contains("`$`"));
        assert!(wrong(&"(".repeat(40)).contains("nests too deep"));
    }
}
