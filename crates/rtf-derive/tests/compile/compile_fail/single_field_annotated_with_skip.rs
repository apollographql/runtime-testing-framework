use rtf_config::templating::Field;
use rtf_derive::Template;

#[derive(Debug, Template)]
struct MyStruct {
    #[template(skip)]
    foo: Field<String>,
}

fn main() {}
