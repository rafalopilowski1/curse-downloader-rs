extern crate futures;
extern crate kuchiki;
extern crate md5;
extern crate regex;
extern crate reqwest;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;
extern crate tendril;
extern crate tokio;

use core::borrow::BorrowMut;
use std::env;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use reqwest::r#async::*;
use tendril::fmt::Slice;
use tendril::stream::TendrilSink;
use tokio::prelude::future::Either;
use tokio::prelude::*;

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
    let file = File::open(path).expect("opening manifest.json failed!");
    println!("Manifest opened!");
    let buf_reader = BufReader::new(file);
    let client = Client::new();
    let mod_pack: Modpack = serde_json::from_reader(buf_reader).unwrap();
    println!("Gathered data. Downloading modfiles...");
    tokio::run(futures::lazy(move || {
        for modfile in mod_pack.files {
            tokio::spawn(
                send_request(modfile, &client)
                    .and_then(|(response, url)| send_request_2(url, &client, response))
                    .and_then(|(response, filename)| {
                        gather_md5(response)
                            .and_then(|(md5, response)| compare_bytes(response, filename, md5))
                    })
                    .and_then(|items| {
                        if file.is_some() {
                            let (file_to_read, response_to_read, path_to_read) = items.unwrap();
                            response_to_body(response_to_read)
                                .and_then(|chunk| save_file(file_to_read, path_to_read, chunk))
                                .and_then(|path| println!("{} <- Done!", path))
                        } else {
                            Ok(())
                        }
                    })
                    .map_err(|e| eprintln!("{:?}", e)),
            )
        }
    }));
}

fn send_request(
    mod_file: Modfile,
    client: &Client,
) -> impl Future<Item = (Response, String), Error = reqwest::Error> {
    let mut url = format!(
        "https://minecraft.curseforge.com/projects/{0}/files/{1}/download",
        mod_file.projectID, mod_file.fileID
    );
    client
        .get(&url)
        .send()
        .and_then(|response| Ok((response, url)))
}

fn response_to_body(response: Response) -> impl Future<Item = Chunk, Error = reqwest::Error> {
    response
        .into_body()
        .concat2()
        .from_err::<reqwest::Error>()
        .and_then(|chunk| Ok(chunk))
}

fn save_file(
    mut file: tokio::fs::File,
    path: PathBuf,
    chunk: Chunk,
) -> impl Future<Item = String, Error = std::io::Error> {
    let path_to_show = path.to_str().unwrap().to_string();
    file.write_all(&chunk).and_then(|_| Ok(path_to_show))
}

fn send_request_2(
    url: String,
    client: &Client,
    response_download: Response,
) -> impl Future<Item = (Response, PathBuf), Error = reqwest::Error> {
    let filename = response_download.url().to_file_path().unwrap();
    let url_to_analyze = url.replace(filename.to_str().unwrap(), "");
    client
        .get(&url_to_analyze)
        .send()
        .and_then(|response| Ok((response, filename)))
}

fn compare_bytes(
    response_download: Response,
    filename: PathBuf,
    md5: String,
) -> impl Future<Item = (), Error = ()> {
    tokio::fs::OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open(&filename)
        .and_then(|file| file.metadata())
        .and_then(|(file, metadata)| {
            if metadata.len() == 0 {
                Either::A(Ok(None))
            }
            let computed_md5 = md5::compute(response_download).to_ascii_lowercase();
            if md5.as_bytes() == computed_md5.as_bytes() {
                Either::A(Ok(None))
            } else {
                Either::B(Ok(Some((file, response_download, filename))))
            }
        })
}

fn gather_md5(
    response: Response,
) -> impl Future<Item = (String, Response), Error = reqwest::Error> {
    response
        .into_body()
        .concat2()
        .from_err()
        .and_then(|mut chunk| {
            let document = kuchiki::parse_html()
                .from_utf8()
                .read_from(chunk.bytes().borrow_mut())
                .unwrap();
            let md5 = document.select_first(".md5").unwrap().text_contents();
            Ok((md5, response))
        })
}
