use rtf_config::templating::Field;
use rtf_derive::Template;

#[derive(Debug, Template)]
struct Example {
    foo: Field<String>,
    bar: Bar,
}

#[derive(Debug, Template)]
struct Bar {
    inner: String,
}

fn main() {}
