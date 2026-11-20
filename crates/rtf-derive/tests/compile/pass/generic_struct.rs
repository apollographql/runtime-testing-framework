use rtf_config::templating::Field;
use rtf_derive::Template;

#[derive(Debug, Template)]
struct MyStruct<T> {
    foo: T,
    bar: Vec<T>,
    baz: Field<String>,
}
