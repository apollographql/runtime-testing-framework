use rtf_config::templating::Field;
use rtf_derive::Template;

#[derive(Debug, Template)]
enum MyEnum {
    Foo(Field<String>),
    Bar(Field<String>),
}
