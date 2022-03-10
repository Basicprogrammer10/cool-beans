use std::fs;
use std::sync::{mpsc, Arc};
use std::thread;

use afire::{
    internal::common::{decode_url, remove_address_port},
    middleware::{MiddleRequest, Middleware},
    Content, Method, Query, Request, Response, ServeStatic, Server,
};
use base64;
use lettre::{
    message, transport::smtp::authentication::Credentials, Message, SmtpTransport, Transport,
};
use parking_lot::Mutex;
use rand::prelude::*;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use simple_config_parser::Config;

struct SimpleLogger;

impl Middleware for SimpleLogger {
    fn pre(&self, req: Request) -> MiddleRequest {
        println!(
            "🌳 [{}] {} {}",
            remove_address_port(req.address),
            req.method,
            req.path
        );
        MiddleRequest::Continue
    }
}

fn main() {
    let config = Config::new().file("config/config.cfg").unwrap();
    let host = config.get_str("host").unwrap();
    let port = config.get("port").unwrap();
    let admin_pass = config.get_str("admin_pass").unwrap();
    let site = config.get_str("site").unwrap();
    let database = config.get_str("database").unwrap();
    let server = config.get_str("server").unwrap();
    let email = config.get_str("email").unwrap();
    let login = config.get_str("login").unwrap();
    let password = config.get_str("password").unwrap();

    let conn = Connection::open(database).unwrap();
    conn.execute(include_str!("sql/init.sql"), []).unwrap();
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    conn.pragma_update(None, "synchronous", "NORMAL").unwrap();
    let pub_conn = Arc::new(Mutex::new(conn));

    let (tx, rx) = mpsc::sync_channel(16);
    thread::Builder::new()
        .name("Mail Thread".to_owned())
        .spawn(move || {
            let mailer = SmtpTransport::relay(&server)
                .unwrap()
                .credentials(Credentials::new(login.to_owned(), password.to_owned()))
                .build();

            for i in rx.iter() {
                let sent = mailer.send(&i).is_ok();
                println!(
                    "📧 Sent to {} [{}]",
                    i.envelope().to().get(0).unwrap(),
                    if sent { "Success" } else { "Fail" }
                )
            }
        })
        .unwrap();

    let mut server = Server::new(host, port);
    ServeStatic::new("web/static").attach(&mut server);
    SimpleLogger.attach(&mut server);

    let conn = pub_conn.clone();
    server.route(Method::POST, "/checkout", move |req| {
        let body = Query::from_body(req.body_string().unwrap()).unwrap();
        let name = decode_url(body.get("name").unwrap());
        let beans = decode_url(body.get("beans").unwrap());
        let ssn = decode_url(body.get("ssn").unwrap());
        let this_email = decode_url(body.get("email").unwrap());

        let bean_int = beans.parse::<u32>().unwrap();
        let code = rand::thread_rng()
            .sample_iter(&rand::distributions::Alphanumeric)
            .take(6)
            .collect::<Vec<u8>>();
        let code = String::from_utf8(code).unwrap();

        let template = fs::read_to_string("web/template/checkout.html").unwrap();
        let body = fs::read_to_string("web/template/tracking_email.html").unwrap();

        conn.lock()
            .execute(
                include_str!("sql/add_bean_buyer.sql"),
                params![code, name, bean_int, email, ssn, 1_u8],
            )
            .unwrap();

        let template = template
            .replace("{{NAME}}", &name)
            .replace("{{BEANS}}", &beans)
            .replace("{{EMAIL}}", &this_email);
        let body = body
            .replace("{{NAME}}", &name)
            .replace("{{BEANS}}", &beans)
            .replace("{{SITE}}", &site)
            .replace("{{CODE}}", &code);

        let to_send = Message::builder()
            .from(format!("coolbeans.biz <{}>", email).parse().unwrap())
            .to(format!("{} <{}>", name, this_email).parse().unwrap())
            .subject("Cool bean shipment")
            .header(message::header::ContentType::TEXT_HTML)
            .body(body)
            .unwrap();

        tx.send(to_send).unwrap();
        Response::new().text(template).content(Content::HTML)
    });

    let conn = pub_conn.clone();
    server.route(Method::GET, "/tracking/{code}", move |req| {
        let code = req.path_param("code").unwrap();
        let file = fs::read_to_string("web/template/tracking.html").unwrap();

        let query: (String, u32, u8) = conn
            .lock()
            .query_row(
                "SELECT name, beans, bean_stats FROM bean_buyer WHERE id = ?1",
                [&code],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();

        let mut file = file
            .replace("{{CODE}}", &code)
            .replace("{{NAME}}", &query.0)
            .replace("{{BEANS}}", &query.1.to_string())
            .replace(
                "{{BEAN_STATUS}}",
                match query.2 {
                    1 => "Shipped",
                    2 => "In Transit",
                    3 => "Delivered",
                    _ => "*COOLBEANS",
                },
            );

        for (i, e) in ["HIDDEN_SHIP", "HIDDEN_TRAN", "HIDDEN_DLIV"]
            .iter()
            .enumerate()
        {
            let rep = if i + 1 <= query.2 as usize {
                ""
            } else {
                "grey"
            };
            file = file.replace(&format!("{{{{{}}}}}", e), rep);
        }

        Response::new().text(file)
    });

    let conn = pub_conn.clone();
    server.route(Method::GET, "/admin", move |req| {
        let auth = req.header("Authorization");
        if auth.is_none() {
            return Response::new()
                .status(401)
                .header("WWW-Authenticate", "Basic realm=\"User Visible Realm\"");
        }

        let auth = auth.unwrap();
        let pass = auth.split(" ").nth(1).unwrap();
        let pass = String::from_utf8(base64::decode(pass).unwrap()).unwrap();
        let pass = pass.split(':').nth(1).unwrap();

        let mut hasher = Sha256::default();
        hasher.update(pass.as_bytes());
        if format!("{:x}", hasher.finalize()) != admin_pass {
            return Response::new()
                .status(400)
                .text("Invalid Admin Password")
                .content(Content::TXT);
        }

        let mut res = String::new();
        let conn = conn.lock();

        if let Some(i) = req.query.get("fore") {
            conn.execute(
                "UPDATE bean_buyer SET bean_stats = bean_stats + 1 WHERE id = ?1",
                [i],
            )
            .unwrap();
        }

        if let Some(i) = req.query.get("back") {
            conn.execute(
                "UPDATE bean_buyer SET bean_stats = bean_stats - 1 WHERE id = ?1",
                [i],
            )
            .unwrap();
        }

        if let Some(i) = req.query.get("del") {
            conn.execute("DELETE FROM bean_buyer WHERE id = ?1", [i])
                .unwrap();
        }

        let mut query = conn.prepare("SELECT * FROM bean_buyer").unwrap();
        let bean_buyers = query
            .query_map([], |row| {
                let item: (String, String, u32, String, String, u8) = (
                    row.get("id")?,
                    row.get("name")?,
                    row.get("beans")?,
                    row.get("email")?,
                    row.get("ssn")?,
                    row.get("bean_stats")?,
                );
                Ok(item)
            })
            .unwrap();

        for i in bean_buyers {
            let i = i.unwrap();

            let status = match i.5 {
                1 => "Shipped",
                2 => "In Transit",
                3 => "Delivered",
                _ => "*COOL BEANS*",
            };

            res.push_str(&format!(
                r#"<tr>
                    <td>{id}</td>
                    <td>{}</td>
                    <td>{}</td>
                    <td>{}</td>
                    <td>{}</td>
                    <td>{}</td>
                    <td>
                        <a g href="?del={id}"><i class="fa fa-trash"></i></a>
                        <a g href="/tracking/{id}"><i class="fa fa-external-link"></i></a>
                        <a g href="?back={id}"><i class="fa fa-chevron-left"></i></a>
                        <a g href="?fore={id}"><i class="fa fa-chevron-right"></i></a>
                    </td>
                    </tr>"#,
                i.1,
                i.2,
                i.3,
                i.4,
                status,
                id = i.0
            ));
        }

        let template = fs::read_to_string("web/template/admin.html")
            .unwrap()
            .replace("{{ADMIN}}", &res);

        Response::new().text(template).content(Content::HTML)
    });

    server.start().unwrap();
}
