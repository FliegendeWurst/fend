use std::sync::Arc;

use crate::{Span, ast, error::{IntErr, Interrupt}, lexer::{self, Symbol, Token}, parser, scope::Scope, value::Value};

pub(crate) fn evaluate_to_value<'a, I: Interrupt>(
    input: &'a str,
    scope: Option<Arc<Scope<'a>>>,
    context: &mut crate::Context,
    int: &I,
) -> Result<Value<'a>, IntErr<String, I>> {
    //eprintln!("input {}", input);
    let lex = lexer::lex(input, int);
    let mut tokens = vec![];
    let mut missing_open_parens: i32 = 0;
    for token in lex {
        let token = token.map_err(IntErr::into_string)?;
        if let lexer::Token::Symbol(lexer::Symbol::CloseParens) = token {
            missing_open_parens += 1
        }
        tokens.push(token);
    }
    //eprintln!("tokens pre {:?}", tokens);
    if tokens.len() > 2 {
        let mut i = 1;
        while i < tokens.len() - 1 {
            if matches!(tokens[i - 1], Token::Symbol(Symbol::Div))
                && matches!(tokens[i], Token::Num(_)) && matches!(tokens[i+1], Token::Ident(_)) {
                //eprintln!("inserting stuff @ {}", i);
                tokens.insert(i+2, Token::Symbol(Symbol::CloseParens));
                tokens.insert(i, Token::Symbol(Symbol::OpenParens));
                i += 2;
            }
            i += 1;
        }
    }
    //eprintln!("tokens post {:?}", tokens);
    for _ in 0..missing_open_parens {
        tokens.insert(0, lexer::Token::Symbol(lexer::Symbol::OpenParens));
    }
    let parsed = parser::parse_tokens(&tokens).map_err(|e| e.to_string())?;
    let result = ast::evaluate(parsed, scope, context, int)?;
    Ok(result)
}

pub(crate) fn evaluate_to_spans<'a, I: Interrupt>(
    mut input: &'a str,
    scope: Option<Arc<Scope<'a>>>,
    context: &mut crate::Context,
    int: &I,
) -> Result<Vec<Span>, IntErr<String, I>> {
    let debug = input.strip_prefix("!debug ").map_or(false, |remaining| {
        input = remaining;
        true
    });
    let value = evaluate_to_value(input, scope, context, int)?;
    Ok(if debug {
        vec![Span::from_string(format!("{:?}", value))]
    } else {
        let mut spans = vec![];
        value.format(0, &mut spans, int)?;
        spans
    })
}
