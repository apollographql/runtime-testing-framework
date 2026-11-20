use rtf_config::templating::Field;
use rtf_derive::Template;

#[derive(Debug, Template)]
struct MyStruct<'a, T: Clone, U, const N: usize>
where
    U: std::fmt::Debug,
{
    foo: T,
    bar: U,
    #[template(skip)]
    baz: &'a [Field<String>; N],
}
