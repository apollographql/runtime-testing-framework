use rtf_config::templating::Field;
use rtf_derive::Template;

#[derive(Debug, Template)]
enum MyEnum<T> {
    Foo(Foo<T>),
    Bar(Field<String>),
    Unit,
}

#[derive(Debug, Template)]
struct Foo<T> {
    foo: Vec<T>,
}
