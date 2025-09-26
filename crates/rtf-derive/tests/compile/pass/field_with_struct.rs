use rtf_config::templating::Field;
use rtf_derive::Template;

#[derive(Debug, Template)]
struct MyStruct {
    foo: Foo,
    bar: Field<String>,
}

#[derive(Debug, Template)]
struct Foo {
    inner: Field<String>,
}

fn main() {}
