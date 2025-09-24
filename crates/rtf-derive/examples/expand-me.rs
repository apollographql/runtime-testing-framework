use rtf_config::templating::Field;
use rtf_derive::Template;

#[derive(Template)]
struct MyStruct {
    foo: Field<String>,
    bar: Bar,
    #[template(skip)]
    baz: String,
}

#[derive(Template)]
struct Bar {
    inner: Field<u32>,
}

fn main() {}
