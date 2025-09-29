use rtf_config::templating::Field;
use rtf_derive::Template;

#[derive(Debug, Template)]
struct MyStruct {
    foo: Field<String>,
    bar: Field<bool>,
    baz: Field<f64>,
    qux: Field<i32>,
}
