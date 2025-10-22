use rtf_config::templating::Field;
use rtf_derive::Template;

#[derive(Debug, Template)]
struct MyStruct {
    foo: Foo,
}

#[derive(Debug)]
struct Foo {
    bar: Field<String>,
}

fn main() {}
