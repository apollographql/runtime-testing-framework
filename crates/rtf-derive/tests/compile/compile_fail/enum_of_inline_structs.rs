use rtf_config::templating::Field;
use rtf_derive::Template;

#[derive(Debug, Template)]
enum MyEnum {
    Foo { field: Field<String> },
    Bar(Field<String>),
}

fn main() {}
