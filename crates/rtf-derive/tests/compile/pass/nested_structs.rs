use rtf_config::templating::Field;
use rtf_derive::Template;

#[derive(Debug, Template)]
struct MyStruct {
    foo: Foo,
}

#[derive(Debug, Template)]
struct Foo {
    bar: Bar,
}

#[derive(Debug, Template)]
struct Bar {
    baz: Field<String>,
}

fn main() {}
