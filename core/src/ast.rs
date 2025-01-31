use crate::error::{IntErr, Interrupt, Never};
use crate::eval::evaluate_to_value;
use crate::ident::Ident;
use crate::interrupt::test_int;
use crate::num::{Base, FormattingStyle, Number};
use crate::scope::{GetIdentError, Scope};
use crate::value::{ApplyMulHandling, BuiltInFunction, Value};
use std::borrow;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub(crate) enum Expr<'a> {
    Num(Number<'a>),
    String(borrow::Cow<'a, str>),
    Ident(Ident<'a>),
    Parens(Box<Expr<'a>>),
    UnaryMinus(Box<Expr<'a>>),
    UnaryPlus(Box<Expr<'a>>),
    UnaryDiv(Box<Expr<'a>>),
    Factorial(Box<Expr<'a>>),
    Add(Box<Expr<'a>>, Box<Expr<'a>>),
    ImplicitAdd(Box<Expr<'a>>, Box<Expr<'a>>),
    Sub(Box<Expr<'a>>, Box<Expr<'a>>),
    Mul(Box<Expr<'a>>, Box<Expr<'a>>),
    Div(Box<Expr<'a>>, Box<Expr<'a>>),
    Pow(Box<Expr<'a>>, Box<Expr<'a>>),
    // Call a function or multiply the expressions
    Apply(Box<Expr<'a>>, Box<Expr<'a>>),
    // Call a function, or throw an error if lhs is not a function
    ApplyFunctionCall(Box<Expr<'a>>, Box<Expr<'a>>),
    // Multiply the expressions
    ApplyMul(Box<Expr<'a>>, Box<Expr<'a>>),

    As(Box<Expr<'a>>, Box<Expr<'a>>),
    Fn(Ident<'a>, Box<Expr<'a>>),

    Of(Ident<'a>, Box<Expr<'a>>),
}

impl<'a> Expr<'a> {
    pub(crate) fn format<I: Interrupt>(&self, int: &I) -> Result<String, IntErr<Never, I>> {
        Ok(match self {
            Self::Num(n) => n.format(int)?.to_string(),
            Self::String(s) => format!(r#""{}""#, s.as_ref()),
            Self::Ident(ident) => ident.to_string(),
            Self::Parens(x) => format!("({})", x.format(int)?),
            Self::UnaryMinus(x) => format!("(-{})", x.format(int)?),
            Self::UnaryPlus(x) => format!("(+{})", x.format(int)?),
            Self::UnaryDiv(x) => format!("(/{})", x.format(int)?),
            Self::Factorial(x) => format!("{}!", x.format(int)?),
            Self::Add(a, b) | Self::ImplicitAdd(a, b) => {
                format!("({}+{})", a.format(int)?, b.format(int)?)
            }
            Self::Sub(a, b) => format!("({}-{})", a.format(int)?, b.format(int)?),
            Self::Mul(a, b) => format!("({}*{})", a.format(int)?, b.format(int)?),
            Self::Div(a, b) => format!("({}/{})", a.format(int)?, b.format(int)?),
            Self::Pow(a, b) => format!("({}^{})", a.format(int)?, b.format(int)?),
            Self::Apply(a, b) => format!("({} ({}))", a.format(int)?, b.format(int)?),
            Self::ApplyFunctionCall(a, b) | Self::ApplyMul(a, b) => {
                format!("({} {})", a.format(int)?, b.format(int)?)
            }
            Self::As(a, b) => format!("({} as {})", a.format(int)?, b.format(int)?),
            Self::Fn(a, b) => {
                if a.as_str().contains('.') {
                    format!("({}:{})", a, b.format(int)?)
                } else {
                    format!("\\{}.{}", a, b.format(int)?)
                }
            }
            Self::Of(a, b) => format!("{} of {}", a, b.format(int)?),
        })
    }
}

/// returns true if rhs is '-1' or '(-1)'
fn should_compute_inverse(rhs: &Expr<'_>) -> bool {
    if let Expr::UnaryMinus(inner) = &*rhs {
        if let Expr::Num(n) = &**inner {
            if n.is_unitless_one() {
                return true;
            }
        }
    } else if let Expr::Parens(inner) = &*rhs {
        if let Expr::UnaryMinus(inner2) = &**inner {
            if let Expr::Num(n) = &**inner2 {
                if n.is_unitless_one() {
                    return true;
                }
            }
        }
    }
    false
}

pub(crate) fn evaluate<'a, I: Interrupt>(
    expr: Expr<'a>,
    scope: Option<Arc<Scope<'a>>>,
    context: &mut crate::Context,
    int: &I,
) -> Result<Value<'a>, IntErr<String, I>> {
    macro_rules! eval {
        ($e:expr) => {
            evaluate($e, scope.clone(), context, int)
        };
    }
    test_int(int)?;
    Ok(match expr {
        Expr::<'a>::Num(n) => Value::Num(n),
        Expr::<'a>::String(s) => Value::String(s),
        Expr::<'a>::Ident(ident) => resolve_identifier(ident, scope, context, int)?,
        Expr::<'a>::Parens(x) => eval!(*x)?,
        Expr::<'a>::UnaryMinus(x) => eval!(*x)?.handle_num(|x| Ok(-x), Expr::UnaryMinus, scope)?,
        Expr::<'a>::UnaryPlus(x) => eval!(*x)?.handle_num(Ok, Expr::UnaryPlus, scope)?,
        Expr::<'a>::UnaryDiv(x) => eval!(*x)?.handle_num(
            |x| Number::from(1).div(x, int).map_err(IntErr::into_string),
            Expr::UnaryDiv,
            scope,
        )?,
        Expr::<'a>::Factorial(x) => {
            eval!(*x)?.handle_num(|x| x.factorial(int), Expr::Factorial, scope)?
        }
        Expr::<'a>::Add(a, b) | Expr::<'a>::ImplicitAdd(a, b) => {
            evaluate_add(eval!(*a)?, eval!(*b)?, scope, int)?
        }
        Expr::<'a>::Sub(a, b) => {
            let a = eval!(*a)?;
            match a {
                Value::Num(a) => Value::Num(a.sub(eval!(*b)?.expect_num()?, int)?),
                f @ Value::BuiltInFunction(_) | f @ Value::Fn(_, _, _) => f.apply(
                    Expr::<'a>::UnaryMinus(b),
                    ApplyMulHandling::OnlyApply,
                    scope,
                    context,
                    int,
                )?,
                _ => return Err("invalid operands for subtraction".to_string().into()),
            }
        }
        Expr::<'a>::Mul(a, b) => eval!(*a)?.handle_two_nums(
            eval!(*b)?,
            |a, b| a.mul(b, int).map_err(IntErr::into_string),
            |a| |f| Expr::Mul(f, Box::new(Expr::Num(a))),
            |a| |f| Expr::Mul(Box::new(Expr::Num(a)), f),
            scope,
        )?,
        Expr::<'a>::Apply(a, b) | Expr::<'a>::ApplyMul(a, b) => {
            if let (Expr::Ident(a), Expr::Ident(b)) = (&*a, &*b) {
                let ident = format!("{}_{}", a, b);
                if let Ok(val) = crate::units::query_unit_static(&ident, context, int) {
                    return Ok(val);
                }
            }
            eval!(*a)?.apply(*b, ApplyMulHandling::Both, scope, context, int)?
        }
        Expr::<'a>::Div(a, b) => eval!(*a)?.handle_two_nums(
            eval!(*b)?,
            |a, b| a.div(b, int).map_err(IntErr::into_string),
            |a| |f| Expr::Div(f, Box::new(Expr::Num(a))),
            |a| |f| Expr::Div(Box::new(Expr::Num(a)), f),
            scope,
        )?,
        Expr::<'a>::Pow(a, b) => {
            let lhs = eval!(*a)?;
            if should_compute_inverse(&*b.clone()) {
                let result = match &lhs {
                    Value::BuiltInFunction(f) => Some(f.invert()?),
                    Value::Fn(_, _, _) => {
                        return Err("inverses of lambda functions are not currently supported"
                            .to_string()
                            .into())
                    }
                    _ => None,
                };
                if let Some(res) = result {
                    return Ok(res);
                }
            }
            lhs.handle_two_nums(
                eval!(*b)?,
                |a, b| a.pow(b, int),
                |a| |f| Expr::Pow(f, Box::new(Expr::Num(a))),
                |a| |f| Expr::Pow(Box::new(Expr::Num(a)), f),
                scope,
            )?
        }
        Expr::<'a>::ApplyFunctionCall(a, b) => {
            eval!(*a)?.apply(*b, ApplyMulHandling::OnlyApply, scope, context, int)?
        }
        Expr::<'a>::As(a, b) => evaluate_as(*a, *b, scope, context, int)?,
        Expr::<'a>::Fn(a, b) => Value::Fn(a, b, scope),
        Expr::<'a>::Of(a, b) => match eval!(*b)?.get_object_member(a) {
            Ok(value) => value,
            Err(msg) => return Err(msg.to_string().into()),
        },
    })
}

fn evaluate_add<'a, I: Interrupt>(
    a: Value<'a>,
    b: Value<'a>,
    scope: Option<Arc<Scope<'a>>>,
    int: &I,
) -> Result<Value<'a>, IntErr<String, I>> {
    Ok(match (a, b) {
        (Value::Num(a), Value::Num(b)) => Value::Num(a.add(b, int)?),
        (Value::String(a), Value::String(b)) => {
            Value::String(format!("{}{}", a.as_ref(), b.as_ref()).into())
        }
        (Value::BuiltInFunction(f), Value::Num(a)) => {
            f.wrap_with_expr(|f| Expr::Add(f, Box::new(Expr::Num(a))), scope)
        }
        (Value::Num(a), Value::BuiltInFunction(f)) => {
            f.wrap_with_expr(|f| Expr::Add(Box::new(Expr::Num(a)), f), scope)
        }
        (Value::Fn(param, expr, scope), Value::Num(a)) => Value::Fn(
            param,
            Box::new(Expr::Add(expr, Box::new(Expr::Num(a)))),
            scope,
        ),
        (Value::Num(a), Value::Fn(param, expr, scope)) => Value::Fn(
            param,
            Box::new(Expr::Add(Box::new(Expr::Num(a)), expr)),
            scope,
        ),
        _ => return Err("expected a number".to_string().into()),
    })
}

fn evaluate_as<'a, I: Interrupt>(
    a: Expr<'a>,
    b: Expr<'a>,
    scope: Option<Arc<Scope<'a>>>,
    context: &mut crate::Context,
    int: &I,
) -> Result<Value<'a>, IntErr<String, I>> {
    if let Expr::Ident(ident) = &b {
        match ident.as_str() {
            "date" => {
                let a = evaluate(a, scope.clone(), context, int)?;
                return if let Value::String(s) = a {
                    Ok(Value::Date(
                        crate::date::Date::parse(s.as_ref()).map_err(|e| e.to_string())?,
                    ))
                } else {
                    Err("expected a string".to_string().into())
                };
            }
            "string" => {
                return Ok(Value::String(
                    evaluate(a, scope, context, int)?
                        .format_to_plain_string(0, int)?
                        .into(),
                ));
            }
            "codepoint" => {
                let a = evaluate(a, scope.clone(), context, int)?;
                if let Value::String(s) = a {
                    let ch = s
                        .as_ref()
                        .chars()
                        .next()
                        .ok_or_else(|| "string cannot be empty".to_string())?;
                    if s.len() > ch.len_utf8() {
                        return Err("string cannot be longer than one codepoint"
                            .to_string()
                            .into());
                    }
                    let value = Value::Num(Number::from(u64::from(ch as u32)).with_base(Base::HEX));
                    return Ok(value);
                }
                return Err("expected a string".to_string().into());
            }
            _ => (),
        }
    }
    Ok(match evaluate(b, scope.clone(), context, int)? {
        Value::Num(b) => Value::Num(
            evaluate(a, scope, context, int)?
                .expect_num()?
                .convert_to(b, int)?,
        ),
        Value::Format(fmt) => Value::Num(
            evaluate(a, scope, context, int)?
                .expect_num()?
                .with_format(fmt),
        ),
        Value::Dp => {
            return Err(
                "you need to specify what number of decimal places to use, e.g. '10 dp'"
                    .to_string()
                    .into(),
            );
        }
        Value::Sf => {
            return Err(
                "you need to specify what number of significant figures to use, e.g. '10 sf'"
                    .to_string()
                    .into(),
            );
        }
        Value::Base(base) => Value::Num(
            evaluate(a, scope, context, int)?
                .expect_num()?
                .with_base(base),
        ),
        Value::BuiltInFunction(_) | Value::Fn(_, _, _) => {
            return Err("unable to convert value to a function".to_string().into());
        }
        Value::Object(_) => {
            return Err("cannot convert value to object".to_string().into());
        }
        Value::String(_) => {
            return Err("cannot convert value to string".to_string().into());
        }
        Value::Date(_) => {
            return Err("cannot convert value to date".to_string().into());
        }
    })
}

pub(crate) fn resolve_identifier<'a, I: Interrupt>(
    ident: Ident<'a>,
    scope: Option<Arc<Scope<'a>>>,
    context: &mut crate::Context,
    int: &I,
) -> Result<Value<'a>, IntErr<String, I>> {
    macro_rules! eval_box {
        ($input:expr) => {
            Box::new(evaluate_to_value($input, scope.clone(), context, int)?)
        };
    }
    if let Some(scope) = scope.clone() {
        match scope.get(ident.as_str(), context, int) {
            Ok(val) => return Ok(val),
            Err(IntErr::Interrupt(int)) => return Err(IntErr::Interrupt(int)),
            Err(IntErr::Error(GetIdentError::IdentifierNotFound(_))) => (),
            Err(IntErr::Error(err @ GetIdentError::EvalError(_))) => {
                return Err(IntErr::Error(err.to_string()))
            }
        }
    }
    Ok(match ident.as_str() {
        "pi" | "\u{3c0}" => Value::Num(Number::pi()),
        "tau" | "\u{3c4}" => Value::Num(Number::pi().mul(2.into(), int)?),
        "e" => evaluate_to_value("approx. 2.718281828459045235", scope, context, int)?,
        "i" => Value::Num(Number::i()),
        "sqrt" => evaluate_to_value("x: x^(1/2)", scope, context, int)?,
        "cbrt" => evaluate_to_value("x: x^(1/3)", scope, context, int)?,
        "conjugate" => Value::BuiltInFunction(BuiltInFunction::Conjugate),
        "abs" => Value::BuiltInFunction(BuiltInFunction::Abs),
        "sin" => Value::BuiltInFunction(BuiltInFunction::Sin),
        "cos" => Value::BuiltInFunction(BuiltInFunction::Cos),
        "tan" => Value::BuiltInFunction(BuiltInFunction::Tan),
        "asin" => Value::BuiltInFunction(BuiltInFunction::Asin),
        "acos" => Value::BuiltInFunction(BuiltInFunction::Acos),
        "atan" => Value::BuiltInFunction(BuiltInFunction::Atan),
        "sinh" => Value::BuiltInFunction(BuiltInFunction::Sinh),
        "cosh" => Value::BuiltInFunction(BuiltInFunction::Cosh),
        "tanh" => Value::BuiltInFunction(BuiltInFunction::Tanh),
        "asinh" => Value::BuiltInFunction(BuiltInFunction::Asinh),
        "acosh" => Value::BuiltInFunction(BuiltInFunction::Acosh),
        "atanh" => Value::BuiltInFunction(BuiltInFunction::Atanh),
        "cis" => evaluate_to_value("theta => cos theta + i * sin theta", scope, context, int)?,
        "ln" => Value::BuiltInFunction(BuiltInFunction::Ln),
        "log2" => Value::BuiltInFunction(BuiltInFunction::Log2),
        "log" | "log10" => Value::BuiltInFunction(BuiltInFunction::Log10),
        "exp" => evaluate_to_value("x: e^x", scope, context, int)?,
        "approx." | "approximately" => Value::BuiltInFunction(BuiltInFunction::Approximately),
        "auto" => Value::Format(FormattingStyle::Auto),
        "exact" => Value::Format(FormattingStyle::Exact),
        "frac" | "fraction" => Value::Format(FormattingStyle::ImproperFraction),
        "mixed_fraction" => Value::Format(FormattingStyle::MixedFraction),
        "float" => Value::Format(FormattingStyle::ExactFloat),
        "dp" => Value::Dp,
        "sf" => Value::Sf,
        "base" => Value::BuiltInFunction(BuiltInFunction::Base),
        "dec" | "decimal" => Value::Base(Base::from_plain_base(10).map_err(|e| e.to_string())?),
        "hex" | "hexadecimal" => Value::Base(Base::from_plain_base(16).map_err(|e| e.to_string())?),
        "binary" => Value::Base(Base::from_plain_base(2).map_err(|e| e.to_string())?),
        "oct" | "octal" => Value::Base(Base::from_plain_base(8).map_err(|e| e.to_string())?),
        "version" => Value::String(crate::get_version_as_str().into()),
        "square" => evaluate_to_value("x: x^2", scope, context, int)?,
        "cubic" => evaluate_to_value("x: x^3", scope, context, int)?,
        "earth" => Value::Object(vec![
            ("axial_tilt", eval_box!("23.4392811 degrees")),
            ("eccentricity", eval_box!("0.0167086")),
            ("escape_velocity", eval_box!("11.186 km/s")),
            ("gravity", eval_box!("9.80665 m/s^2")),
            ("mass", eval_box!("5.97237e24 kg")),
            ("volume", eval_box!("1.08321e12 km^3")),
        ]),
        "differentiate" => Value::BuiltInFunction(BuiltInFunction::Differentiate),
        "today" => Value::Date(crate::date::Date::today(context).map_err(|e| e.to_string())?),
        "tomorrow" => Value::Date(
            crate::date::Date::today(context)
                .map_err(|e| e.to_string())?
                .next(),
        ),
        "yesterday" => Value::Date(
            crate::date::Date::today(context)
                .map_err(|e| e.to_string())?
                .prev(),
        ),
        _ => {
            return crate::units::query_unit(ident.as_str(), context, int)
                .map_err(IntErr::into_string)
        }
    })
}
