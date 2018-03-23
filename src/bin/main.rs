extern crate httpd;
extern crate percent_encoding;

use httpd::ThreadPool;
use percent_encoding as pe;
use std::borrow::Borrow;
use std::error::Error;
use std::fs;
use std::io::{BufReader, BufWriter};
use std::io::prelude::*;
use std::net::TcpListener;
use std::net::TcpStream;
use std::path::{Path, PathBuf};

const GENERATE_INDEXES: bool = true;

fn main() {
	let listener = TcpListener::bind("[::]:8080").unwrap();
	let pool = ThreadPool::new(4);
	
	for stream in listener.incoming() {
		let stream = stream.unwrap();
		
		#[allow(unused_must_use)]
		pool.execute(move || {
			if let Err(_) = handle_connection(&stream) {
				let mut writer = BufWriter::new(&stream);
				abort(&mut writer, 500); // result ignored
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

	let mut fs_path = path;
	fs_path.insert_str(0, ".");
	let fs_path = Path::new(&fs_path);

	if !fs_path.exists() {
		abort(&mut writer, 404)?;
		return Ok(());
	}

	let mut fs_path = PathBuf::from(&fs_path).canonicalize()?;

	let md = fs::metadata(&fs_path)?;
	if md.is_dir() {
		if !http_path.ends_with('/') {
			http_path.push('/');
			redirect(&mut writer, 301, PathBuf::from(http_path))?;
			return Ok(());
		}

		let mut index_path = fs_path.clone();
		index_path.push("index.html");

		if !index_path.exists() {
			if GENERATE_INDEXES {
				write_index(&mut writer, &fs_path, &http_path)?;
			} else {
				abort(&mut writer, 404)?;
			}
			return Ok(());
		}

		fs_path = index_path;
	}

	let mut file = fs::File::open(&fs_path)?;
	let mut content = String::new();
	let mut headers: Vec<(&str, &str)> = vec![];
	let content_type = guess_content_type(&fs_path);
	headers.push(("Content-Type", &content_type));
	file.read_to_string(&mut content)?;
	write_content(&mut writer, headers, &content)?;
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

fn abort(mut writer: &mut BufWriter<&TcpStream>, code: u32) -> Result<(), Box<Error + Send + Sync>> {
	let reason = match code {
		404 => "Not Found",
		405 => "Method Not Allowed",
		505 => "HTTP Version Not Supported",
		500 | _ => "Internal Server Error",
	};
	
	let mut content = String::new();
	let mut headers: Vec<(&str, &str)> = vec![];
	let filename = format!("{}.html", code);
	if let Ok(mut file) = fs::File::open(filename) {
		headers.push(("Content-Type", "text/html"));
		file.read_to_string(&mut content)?;
	} else {
		headers.push(("Content-Type", "text/plain"));
		content.push_str(reason);
	}
	write_content(&mut writer, headers, &content)?;
	Ok(())
}

fn write_index(mut writer: &mut BufWriter<&TcpStream>, fs_path: &Path, http_path: &str) -> Result<(), Box<Error + Send + Sync>> {
	let mut content = String::new();
	let headers: Vec<(&str, &str)> = vec![("Content-Type", "text/html")];
	content.push_str(&format!("<html><head><title>Index of {}</title></head><body><h1>Index of {}</h1>", http_path, http_path));

	if http_path != "/" {
		content.push_str(&format!("<a href=\"{}../\">../</a><br/>", http_path));
	}

	let mut paths: Vec<_> = fs::read_dir(fs_path)
		.unwrap().map(|r| r.unwrap()).collect();
	paths.sort_by_key(|dir| (!dir.path().is_dir(), dir.path()));
	for entry in paths {
		let path = entry.path();
		let name = path.strip_prefix(fs_path)?;
		if path.is_dir() {
			content.push_str(&format!("<a href=\"{}{}/\">{}/</a><br/>", http_path, name.display(), name.display()));
		} else {
			content.push_str(&format!("<a href=\"{}{}\">{}</a><br/>", http_path, name.display(), name.display()));
		}
	}

	content.push_str("<body></html>");
	write_content(&mut writer, headers, &content)?;
	Ok(())
}

fn write_content(writer: &mut BufWriter<&TcpStream>, headers: Vec<(&str, &str)>, content: &str) -> Result<(), Box<Error + Send + Sync>> {
	writer.write("HTTP/1.1 200 OK\r\n".as_bytes())?;
	for (name, value) in headers {
		writer.write(format!("{}: {}\r\n", name, value).as_bytes())?;
	}
	writer.write("\r\n".as_bytes())?;
	writer.write(content.as_bytes())?;
	writer.flush()?;
	Ok(())
}

fn guess_content_type(path: &PathBuf) -> &str {
	let extension = match path.extension() {
		Some(extension) => String::from(extension.to_string_lossy()),
		None => String::new(),
	};

	match extension.as_ref() {
		"css" => "text/css",
		"gif" => "image/gif",
		"html" => "text/html",
		"ico" => "image/x-icon",
		"jpg" | "jpeg" => "image/jpeg",
		"js" => "application/javascript",
		"png" => "image/png",
		"svg" => "image/svg+xml",
		"ttf" => "font/ttf",
		"txt" => "text/plain",
		"woff" => "font/woff",
		"woff2" => "font/woff2",
		"xml" => "application/xml",
		_ => "application/octet-stream",
	}
}