extern crate futures;
extern crate kuchiki;
extern crate regex;
extern crate reqwest;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;
extern crate tendril;
extern crate tokio;

use core::borrow::Borrow;
use std::borrow::Cow;
use std::env;
use std::error::Error;
use std::io::{BufReader, ErrorKind};
use std::sync::Arc;

use regex::Regex;
use reqwest::r#async::*;
use tendril::fmt::Slice;
use tokio::fs::File;
use tokio::prelude::*;

use crate::futures::Stream;

#[derive(Deserialize)]
struct Modpack {
    files: Vec<Modfile>,
}

#[derive(Deserialize)]
struct Modfile {
    projectID: serde_json::Number,
    fileID: serde_json::Number,
}

fn main() {
    let args: Vec<_> = env::args().collect();
    let path: &str = &args[1];
    let file = std::fs::File::open(path).expect("opening manifest.json failed!");
    println!("Manifest opened!");
    if let Err(e) = std::fs::create_dir("mods") {
        match e.kind() {
            std::io::ErrorKind::AlreadyExists => println!("'mods' folder already exists..."),
            _ => eprintln!("Error: {:?}", e.description()),
        }
    }
    let buf_reader = BufReader::new(file);
    let client = Arc::new(Client::new());
    let mod_pack: Modpack = serde_json::from_reader(buf_reader).unwrap();
    println!("Gathered data. Downloading modfiles...");
    tokio::run(futures::lazy(move || {
        for mod_file in mod_pack.files {
            tokio::spawn(
                download_mod_file(mod_file, client.clone()).and_then(|response| {
                    let url = Cow::from(response.url().as_str());
                    let regex = Regex::new("(?:[^/]+)$").expect("error: creating Regex object");
                    let match_regex = Cow::from(format!(
                        "./mods/{}",
                        regex
                            .find(url.borrow())
                            .expect("error: url not found - bad regex syntax")
                            .as_str(),
                    ));
                    let body = response.into_body();
                    create_file(match_regex.clone())
                        .and_then(|file| save_file(file, body))
                        .and_then(move |_| done_print(match_regex))
                        .and_then(|_| Ok(()))
                }),
            );
        }
        Ok(())
    }));
}

fn done_print(path: Cow<str>) -> impl Future<Item = (), Error = ()> {
    println!("{} -> Done!", path);
    Ok(()).into_future()
}

fn create_file(path: Cow<str>) -> impl Future<Item = std::fs::File, Error = ()> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(path.borrow() as &str);
    file.into_future().map_err(|e| eprintln!("{:?}", e))
}

fn download_mod_file(
    mod_file: Modfile,
    client: Arc<Client>,
) -> impl Future<Item = Response, Error = ()> {
    let url_to_download = Cow::from(format!(
        "https://www.curseforge.com/minecraft/modpacks/{0}/download/{1}/file",
        mod_file.projectID, mod_file.fileID
    ));
    client
        .get(url_to_download.borrow() as &str)
        .send()
        .and_then(Ok)
        .map_err(|e| eprintln!("{:?}", e))
}

pub fn save_file(file: std::fs::File, body: Decoder) -> impl Future<Item = (), Error = ()> {
    let file = tokio::fs::File::from_std(file);
    let (_, writer) = file.split();
    let stream = body
        .from_err::<reqwest::Error>()
        .map_err(|e| std::io::Error::new(ErrorKind::Other, e));

    let sink = tokio::codec::FramedWrite::new(writer, tokio::codec::BytesCodec::new())
        .with(|byte: reqwest::r#async::Chunk| Ok::<_, std::io::Error>(byte.as_bytes()[..].into()));
    sink.send_all(stream)
        .and_then(|_| Ok(()))
        .map_err(|e| eprintln!("{}", e))
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::io::Read;

    #[test]
    fn create_dir_and_file() {
        if let Err(e) = std::fs::create_dir("mods") {
            match e.kind() {
                std::io::ErrorKind::AlreadyExists => (),
                _ => panic!("Error: {:?}", e.description()),
            }
        }
        if let Err(e) = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open("./mods/test.jar")
        {
            panic!("Error: {:?}", e.description())
        }
    }

}
