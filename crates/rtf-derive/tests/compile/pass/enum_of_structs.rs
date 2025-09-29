use rtf_config::templating::Field;
use rtf_derive::Template;

#[derive(Debug, Template)]
enum MyEnum {
    Foo(Foo),
    Bar(Bar),
}

#[derive(Debug, Template)]
struct Foo {
    foo: Field<String>,
}

#[derive(Debug, Template)]
struct Bar {
    bar: Field<String>,
}
