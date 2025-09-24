use rtf_config::templating::{Field, IterFields};
use rtf_derive::IterFields;

#[derive(IterFields)]
struct MyStruct {
    foo: Field<String>,
    bar: Bar,
}

#[derive(IterFields)]
struct Bar {
    baz: Field<u32>,
}

fn main() {}
