use std::{fs, io::{BufReader, prelude::*}, net::{TcpListener, TcpStream}};

fn main() {
    let listener = TcpListener::bind("127.0.0.1:3000").unwrap();

    for stream in listener.incoming() {
        let stream = stream.unwrap();

        handle_connection(stream);
    }
}

fn handle_connection(mut stream: TcpStream) {
    let buf_reader = BufReader::new(&stream);
    let http_request: Vec<_> = buf_reader
        .lines()
        .map(|result| result.unwrap())
        .take_while(|line| !line.is_empty())
        .collect();

    let status_line = "HTTP/1.1 200 OK";
    let contents = r#"
    {"products": [{
      "id": 1,
      "name": "Lunar Rover Wheels",
      "createdAt": 1636742972000,
      "updatedAt": 1636742972000,
      "description": "Designed for traversing the rugged terrain of the moon, these wheels provide unrivaled traction and durability. Made from a lightweight composite, they ensure your rover is agile in challenging conditions.",
      "slug": "lunar-rover-wheels",
      "tags": [
        {
          "tagId": "space",
          "name": "Space"
        },
        {
          "tagId": "engineering",
          "name": "Engineering"
        },
        {
          "tagId": "rover",
          "name": "Rover"
        }
      ],
      "category": "Engineering Components",
      "availability": "AVAILABLE"
    }]}
    "#;
    let length = contents.len();

    let response =
        format!("{status_line}\r\nContent-Length: {length}\r\n\r\n{contents}");

    stream.write_all(response.as_bytes()).unwrap();
}