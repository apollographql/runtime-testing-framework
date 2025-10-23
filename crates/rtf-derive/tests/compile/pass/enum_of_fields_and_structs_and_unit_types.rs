use rtf_config::templating::Field;
use rtf_derive::Template;

#[derive(Debug, Template)]
enum MyEnum {
    Foo(Foo),
    Bar(Field<String>),
    Baz,
}

#[derive(Debug, Template)]
struct Foo {
    foo: Field<String>,
}
