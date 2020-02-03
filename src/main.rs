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

use std::{
    borrow::Cow,
    env,
    error::Error,
    io::{BufReader, BufWriter, ErrorKind},
    sync::Arc,
};

use reqwest::r#async::*;
use tendril::fmt::Slice;
use tokio::prelude::*;

use crate::futures::Stream;

#[derive(Deserialize)]
struct Modpack {
    files: Vec<Modfile>,
}

#[derive(Deserialize, Clone)]
struct Modfile {
    projectID: serde_json::Number,
    fileID: serde_json::Number,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args().collect();
    let path: &str = &args[1];
    let file = std::fs::File::open(path)?;
    println!("Manifest opened!");
    if let Err(e) = std::fs::create_dir("mods") {
        match e.kind() {
            std::io::ErrorKind::AlreadyExists => println!("'mods' folder already exists..."),
            _ => eprintln!("Error: {:?}", e.description()),
        }
    }
    let buf_reader = BufReader::new(file);
    let client = Arc::new(
        Client::builder()
            .cookie_store(true)
            .gzip(true)
            .tcp_nodelay()
            .build()
            .unwrap(),
    );
    println!("{:#?}", client);
    let mod_pack: Modpack = serde_json::from_reader(buf_reader)?;
    println!("Gathered data. Downloading modfiles...");
    tokio::run(
        stream::iter_ok(mod_pack.files)
            .take(1)
            .for_each(move |mod_file| {
                let client = client.clone();
                tokio::spawn(
                    get_cookie(client.clone(), mod_file.clone())
                        .and_then(move |cookie| {
                            println!("{:#?}", &cookie);
                            download_mod_file(mod_file, client, cookie)
                        })
                        .and_then(|response| {
                            println!("{:#?}", &response);
                            let url: String = "./mods/file.html".parse().unwrap();
                            let body = response.into_body();
                            create_file(url.clone())
                                .and_then(|file| save_file(file, body))
                                .and_then(move |_| done_print(url))
                                .and_then(|_| Ok(()))
                        }),
                );
                Ok(())
            }),
    );

    Ok(())
}

fn get_cookie(client: Arc<Client>, mod_file: Modfile) -> impl Future<Item = Response, Error = ()> {
    let url_to_download = Cow::from(format!(
        "https://minecraft.curseforge.com/projects/{0}",
        mod_file.projectID
    ));

    client
        .get(url_to_download.as_ref())
        .send()
        .and_then(Ok)
        .map_err(|e| eprintln!("{:?}", e))
}

fn done_print<S>(path: S) -> impl Future<Item = (), Error = ()>
where
    S: std::fmt::Display,
{
    println!("{} -> Done!", path);
    Ok(()).into_future()
}

fn create_file<S>(path: S) -> impl Future<Item = std::fs::File, Error = ()>
where
    S: AsRef<str>,
{
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(path.as_ref());
    file.into_future().map_err(|e| eprintln!("{:?}", e))
}

fn download_mod_file(
    mod_file: Modfile,
    client: Arc<Client>,
    cookie: Response,
) -> impl Future<Item = Response, Error = ()> {
    let url_to_download = Cow::from(format!(
        "{0}/download/{1}/file",
        cookie.url().as_str(),
        mod_file.fileID
    ));
    let request = client.get(url_to_download.as_ref()).build().unwrap();
    println!("{:#?}", request);
    client
        .execute(request)
        .and_then(Ok)
        .map_err(|e| eprintln!("{:?}", e))
}

pub fn save_file(file: std::fs::File, body: Decoder) -> impl Future<Item = (), Error = ()> {
    // creating file
    let file = tokio::fs::File::from_std(file);
    // getting write-only access
    let (_, writer) = file.split();
    // buffering chunks of data (less sys-calls)
    let writer = BufWriter::new(writer);
    // unifying all types of `reqwest::Error` in stream of futures from HTTP client response stream to `io::Error(ErrorKind::Other)` (temporary measure)
    let stream = body
        .from_err::<reqwest::Error>()
        .map_err(|error| std::io::Error::new(ErrorKind::Other, error));
    // creating async writes via `tokio::codec::FramedWrite`, which debundles bytes from Future container
    let sink = tokio::codec::FramedWrite::new(writer, tokio::codec::BytesCodec::new())
        .with(|byte: reqwest::r#async::Chunk| Ok::<_, std::io::Error>(byte.as_bytes()[..].into()));
    // sending all packets of bytes from `Stream` to `Sink` (headed to file)
    sink.send_all(stream)
        .and_then(|_| Ok(()))
        .map_err(|e| eprintln!("{}", e))
}
