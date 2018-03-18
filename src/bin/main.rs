extern crate hello;
extern crate percent_encoding;

use hello::ThreadPool;
use percent_encoding as pe;
use std::borrow::Borrow;
use std::error::Error;
use std::fs::{File, metadata};
use std::io::{BufReader, BufWriter};
use std::io::prelude::*;
use std::net::TcpListener;
use std::net::TcpStream;
use std::path::{Path, PathBuf};

fn main() {
	let listener = TcpListener::bind("[::]:8080").unwrap();
	let pool = ThreadPool::new(4);
	
	for stream in listener.incoming() {
		let stream = stream.unwrap();
		
		#[allow(unused_must_use)]
		pool.execute(move || {
			if let Err(_) = handle_connection(&stream) {
				let mut writer = BufWriter::new(&stream);
				abort(&mut writer, 500); // ignored
			}
		});
	}
	
	println!("Shutting down.");
}

fn handle_connection(stream: &TcpStream) -> Result<(), Box<Error + Send + Sync>> {
	let mut reader = BufReader::new(stream);
	let mut writer = BufWriter::new(stream);

	let mut request = String::new();
	reader.read_line(&mut request)?;

	let request = request.trim_right_matches("\r\n");
	
	let (method, path, _args, version) = parse_request(request)?;

	if version != "HTTP/1.1" {
		abort(&mut writer, 505)?;
		return Ok(());
	} else if method != "GET" {
		abort(&mut writer, 405)?;
		return Ok(());
	}

	loop {
		let mut header = String::new();	
		reader.read_line(&mut header)?;
		let header = header.trim_right_matches("\r\n");
		if header.is_empty() {
			break;
		}
	}

	/* read body here if needed */

	let mut http_path = String::new();
	http_path.push_str(&path);

	let mut path = path;
	path.insert_str(0, ".");
	let path = Path::new(&path);

	if !path.exists() {
		abort(&mut writer, 404)?;
		return Ok(());
	}

	let mut path = PathBuf::from(&path);

	let md = metadata(&path)?;
	if md.is_dir() {
		if !http_path.ends_with('/') {
			http_path.push('/');
			redirect(&mut writer, 301, PathBuf::from(http_path))?;
		}

		path.push("index.html");	
		if !path.exists() {
			/* TODO: Generate index */
			abort(&mut writer, 404)?;
			return Ok(());
		}
	}

	let mut file = File::open(&path)?;
	let mut contents = String::new();
	file.read_to_string(&mut contents)?;
	let response = format!("HTTP/1.1 200 OK\r\n\r\n{}", contents);

	writer.write(response.as_bytes())?;
	writer.flush()?;
	Ok(())
}

fn parse_request(request: &str) -> Result<(&str, String, String, &str), Box<Error + Send + Sync>> {
	let parts: Vec<&str> = request.splitn(3, ' ').collect();
	if parts.len() != 3 {
		return Err(From::from("Invalid request line"));
	}
	let (method, uri, version) = (parts[0], parts[1], parts[2]);
	
	let decoder = pe::percent_decode(uri.as_bytes());
	let decoded = decoder.decode_utf8_lossy();
	let mut uri = String::new();
	uri.push_str(decoded.borrow());
	let parts: Vec<&str> = uri.splitn(2, '?').collect();

	let path = parts[0].to_string();
	let mut args = String::new();
	if parts.len() == 2 {
		args.push_str(parts[1]);
	}

	Ok((method, path, args, version))
}

fn redirect(writer: &mut BufWriter<&TcpStream>, code: u32, path: PathBuf) -> Result<(), Box<Error + Send + Sync>> {
	let reason = match code {
		301 => "Moved Permanently",
		302 => "Found",
		303 => "See Other",
		305 => "Use Proxy",
		307 | _ => "Temporary Redirect",
	};

	let response = format!("HTTP/1.1 {} {}\r\nLocation: {}\r\n\r\n", 
			code, reason, path.to_string_lossy());
	writer.write(response.as_bytes())?;
	writer.flush()?;
	Ok(())
}

fn abort(writer: &mut BufWriter<&TcpStream>, code: u32) -> Result<(), Box<Error + Send + Sync>> {
	let reason = match code {
		404 => "Not Found",
		405 => "Method Not Allowed",
		505 => "HTTP Version Not Supported",
		500 | _ => "Internal Server Error",
	};
	
	let mut body = String::new();
	let filename = format!("{}.html", code);
	if let Ok(mut file) = File::open(filename) {
		file.read_to_string(&mut body)?;
	} else {
		body.push_str(reason);
	}

	let response = format!("HTTP/1.1 {} {}\r\n\r\n{}", code, reason, body);
	writer.write(response.as_bytes())?;
	writer.flush()?;
	Ok(())
}
