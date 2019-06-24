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

use core::borrow::Borrow;
use std::borrow::Cow;
use std::env;
use std::error::Error;
use std::io::{BufReader, ErrorKind};
use std::ops::Add;
use std::sync::Arc;

use reqwest::r#async::*;
use tendril::fmt::Slice;
use tendril::stream::TendrilSink;
use tokio::fs::File;
use tokio::prelude::future::Either;
use tokio::prelude::*;

use crate::futures::Stream;
use crate::tendril::SliceExt;

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
            let client_clone = client.clone();
            tokio::spawn(gather_metadata(mod_file, client_clone.clone()).and_then(
                move |(name_dom, md5, url)| {
                    check_integrity(name_dom, md5).and_then(
                        move |(file_option, file_path_option)| {
                            if file_option.is_some() {
                                let file = file_option.unwrap();
                                let file_path = file_path_option.unwrap();

                                Either::A(
                                    download_mod_file(url, client_clone.clone())
                                        .and_then(move |chunk| save_file(file, chunk))
                                        .and_then(move |_| done_print(file_path))
                                        .and_then(|_| Ok(())),
                                )
                            } else {
                                Either::B(already().and_then(|_| Ok(())))
                            }
                        },
                    )
                },
            ));
        }
        Ok(())
    }));
}

fn done_print(file_path: Cow<str>) -> impl Future<Item = (), Error = ()> {
    println!("{} <- Done!", file_path);
    Ok(()).into_future()
}

fn already() -> impl Future<Item = (), Error = ()> {
    println!("Already downloaded!");
    Ok(()).into_future()
}

fn gather_metadata<'a>(
    mod_file: Modfile,
    client: Arc<Client>,
) -> impl Future<Item = (Cow<'a, str>, Cow<'a, str>, Cow<'a, str>), Error = ()> {
    let url = std::borrow::Cow::from(format!(
        "https://minecraft.curseforge.com/projects/{0}/files/{1}",
        mod_file.projectID, mod_file.fileID
    ));

    client
        .get(url.borrow() as &str)
        .send()
        .and_then(move |response| {
            let error_mes = format!("Error while parsing HTML - {}", &response.url());
            response
                .into_body()
                .from_err()
                .concat2()
                .and_then(move |body| {
                    let bytes = body.to_tendril();
                    let html = kuchiki::parse_html().from_utf8().one(bytes);
                    let name_dom = std::borrow::Cow::from(
                        html.select_first(".info-data.overflow-tip")
                            .expect(&error_mes)
                            .text_contents(),
                    );
                    let md5 = std::borrow::Cow::from(
                        html.select_first(".md5").expect(&error_mes).text_contents(),
                    );
                    Ok((name_dom, md5, url))
                })
        })
        .map_err(|e| eprintln!("{:?}", e))
}

fn check_integrity<'a>(
    name_dom: Cow<'a, str>,
    md5: Cow<'a, str>,
) -> impl Future<Item = (Option<tokio::fs::File>, Option<Cow<'a, str>>), Error = ()> {
    let path = Cow::from(format!("./mods/{}", name_dom));
    tokio::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(path.into_owned())
        .and_then(move |mut file| {
            let mut file_bytes: Vec<u8> = vec![];
            file.read_buf(&mut file_bytes).and_then(move |_| {
                let result = md5::compute(file_bytes);
                if md5 == format!("{:x}", result) {
                    Ok((None, None))
                } else {
                    Ok((Some(file), Some(name_dom)))
                }
            })
        })
        .map_err(|e| eprintln!("{:?}", e))
}

fn download_mod_file(
    url: Cow<str>,
    client: Arc<Client>,
) -> impl Future<Item = Response, Error = ()> {
    let url_to_download = url.add("/download");
    client
        .get(url_to_download.borrow() as &str)
        .send()
        .and_then(Ok)
        .map_err(|e| eprintln!("{:?}", e))
}

pub fn save_file(file: File, response: Response) -> impl Future<Item = (), Error = ()> {
    let (_, writer) = file.split();
    let stream = response
        .into_body()
        .from_err::<reqwest::Error>()
        .map_err(|e| std::io::Error::new(ErrorKind::Other, e));

    let sink = tokio::codec::FramedWrite::new(writer, tokio::codec::BytesCodec::new())
        .with(|byte: reqwest::r#async::Chunk| Ok::<_, std::io::Error>(byte.as_bytes()[..].into()));
    sink.send_all(stream)
        .and_then(|_| Ok(()).into_future())
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

    #[test]
    fn compare_md5() {
        let mut file =
            match std::fs::File::open("target/release/mods/astralsorcery-1.12.2-1.10.12.jar") {
                Ok(f) => f,
                Err(e) => panic!("Error (opening file): {}", e.description()),
            };
        let mut bytes: Vec<u8> = vec![];
        if let Err(e) = file.read_to_end(&mut bytes) {
            panic!("Error (reading file): {}", e.description())
        }

        let result = md5::compute(bytes);
        assert_eq!("92a9714b398f9e31955a9e97ca3d7f34", format!("{:x}", result));
    }
}
