use rtf_derive::Template;

#[derive(Debug, Template)]
struct MyStruct {
    foo: String,
    bar: usize,
}

fn main() {}
