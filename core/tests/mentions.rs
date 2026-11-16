//! Nombrar a alguien con `@`: dónde se ofrece, a quién, y qué se escribe.

use cac_core::{
    comment_placeholder, mention_link, mention_matches, mention_query, mentioned_user, mentions_allowed, offered_mentions, with_mention,
    MentionPerson, MentionQuery, UserSummary,
};

const ANA: &str = "11111111-1111-4111-8111-111111111111";
const LUIS: &str = "22222222-2222-4222-8222-222222222222";

fn team() -> Vec<UserSummary> {
    vec![
        UserSummary { id: ANA.into(), username: "ana.g".into(), name: Some("Ana García".into()) },
        UserSummary { id: LUIS.into(), username: "luis".into(), name: None },
    ]
}

fn at(text: &str) -> Option<MentionQuery> {
    mention_query(text, text.len())
}

fn usernames(people: &[MentionPerson]) -> Vec<&str> {
    people.iter().map(|person| person.username.as_str()).collect()
}

/// En lo que lee un cliente no se ofrece nombrar a nadie, y la caja no lo
/// promete; en una nota interna o en una tarea del equipo, sí.
///
/// Roturas que atrapa: poner el nombre de un compañero delante de un
/// cliente; prometer el `@` donde no hace nada; negarlo en una nota interna
/// de la tarea de un cliente.
#[test]
fn nobody_is_named_where_a_client_reads() {
    assert!(mentions_allowed(false, false) && mentions_allowed(false, true) && mentions_allowed(true, true));
    assert!(!mentions_allowed(true, false));
    assert_eq!(comment_placeholder(true, false), "work:task.writeCommentClient");
    assert_eq!(comment_placeholder(true, true), "work:task.writeComment");
    assert_eq!(comment_placeholder(false, false), "work:task.writeComment");

    assert!(offered_mentions(&team(), "hola @an", 8, false).is_empty());
    assert_eq!(usernames(&offered_mentions(&team(), "hola @an", 8, true)), ["ana.g"]);
    assert!(offered_mentions(&team(), "hola", 4, true).is_empty(), "ofrece sin haber escrito un @");
}

/// El `@` nombra cuando abre palabra y el cursor está detrás de lo escrito.
///
/// Roturas que atrapa: ofrecer gente al escribir un correo; volver a ofrecer
/// sobre una mención ya puesta; seguir ofreciendo tras un espacio; no ofrecer
/// al principio de una línea; mirar el final del texto y no el cursor.
#[test]
fn an_at_names_someone_only_when_it_opens_a_word() {
    assert_eq!(at("@"), Some(MentionQuery { start: 0, query: String::new() }));
    assert_eq!(at("hola @an"), Some(MentionQuery { start: 5, query: "an".into() }));
    assert_eq!(at("uno\n@ana.g"), Some(MentionQuery { start: 4, query: "ana.g".into() }));
    assert_eq!(at("café @lu"), Some(MentionQuery { start: 6, query: "lu".into() }), "cuenta bytes, no letras");

    assert_eq!(at("ana@correo"), None);
    assert_eq!(at("hola @ana y"), None);
    assert_eq!(at("[@ana.g](cac:user/x"), None);
    assert_eq!(at("sin arroba"), None);

    // El cursor a mitad: sólo cuenta lo que tiene detrás.
    assert_eq!(mention_query("hola @ana y más", 9), Some(MentionQuery { start: 5, query: "ana".into() }));
    assert_eq!(mention_query("hola @ana y más", 12), None);
    // Un cursor que no es de este texto no revienta.
    assert_eq!(mention_query("año @a", 2), None);
    assert_eq!(mention_query("@a", 9), None);
}

/// Dentro de código no se nombra a nadie.
///
/// Rotura que atrapa: ofrecer gente a quien escribe `user@host` o una
/// variable en un bloque de código.
#[test]
fn nobody_is_named_inside_code() {
    assert_eq!(at("```\nssh @ro"), None);
    assert_eq!(at("mira `git @ro"), None);
    assert!(at("```\nx\n```\n@ro").is_some(), "tras cerrar el bloque");
    assert!(at("mira `x` @ro").is_some(), "tras cerrar las comillas");
    assert!(at("`otra línea\n@ro").is_some(), "unas comillas sin cerrar no pasan de su línea");
}

/// Se busca por usuario o por nombre, sin mayúsculas ni acentos.
///
/// Roturas que atrapa: no encontrar a «García» escribiendo «garcia»; buscar
/// sólo por el principio; no ofrecer a nadie con el `@` recién escrito;
/// ofrecer a quien no casa; enseñar un nombre vacío de quien no lo ha puesto.
#[test]
fn people_are_found_by_username_or_name() {
    assert_eq!(usernames(&mention_matches(&team(), "")), ["ana.g", "luis"]);
    assert_eq!(usernames(&mention_matches(&team(), "GARCIA")), ["ana.g"]);
    assert_eq!(usernames(&mention_matches(&team(), "ui")), ["luis"]);
    // Y al revés: el acento puesto de más tampoco estorba.
    assert_eq!(usernames(&mention_matches(&team(), "LUÍS")), ["luis"]);
    assert_eq!(usernames(&mention_matches(&team(), "a.g")), ["ana.g"]);
    assert!(mention_matches(&team(), "zzz").is_empty());
    assert_eq!(mention_matches(&team(), "luis")[0].name, "luis");
    assert_eq!(mention_matches(&team(), "ana")[0].name, "Ana García");

    let many: Vec<UserSummary> =
        (0..30).map(|n| UserSummary { id: format!("id-{n}"), username: format!("user{n}"), name: None }).collect();
    assert_eq!(mention_matches(&many, "user").len(), 20);
}

/// Elegir a alguien cambia el `@algo` por su mención, que es la que el
/// servidor reconoce, y deja el cursor detrás para seguir.
///
/// Roturas que atrapa: dejar un `@ana` suelto, que no avisa a nadie;
/// mencionar por el nombre y no por el usuario; comerse lo que había después
/// del cursor; duplicar lo ya escrito del nombre; dejar el cursor en medio.
#[test]
fn choosing_someone_writes_the_mention_the_server_reads() {
    let ana = mention_matches(&team(), "ana").remove(0);
    let link = format!("[@ana.g](cac:user/{ANA})");
    assert_eq!(mention_link(&ana), link);

    let edit = with_mention("hola @an, mira", 8, &ana).unwrap();
    assert_eq!(edit.text, format!("hola {link} , mira"));
    assert_eq!(&edit.text[..edit.cursor as usize], format!("hola {link} "));

    // La forma exacta que busca `domain.ExtractMentions`.
    let id = edit.text.split("cac:user/").nth(1).unwrap();
    assert_eq!(&id[..36], ANA);
    assert_eq!(&id[36..37], ")");

    assert_eq!(with_mention("hola", 4, &ana), None, "sin @ que cambiar");
}

/// Una mención se reconoce por su enlace, con la forma exacta que escribe.
///
/// Roturas que atrapa: tratar una mención como un enlace al que ir; tomar
/// por mención cualquier cosa que empiece igual, o un enlace que la contiene.
#[test]
fn a_mention_is_told_from_any_other_link() {
    let ana = mention_matches(&team(), "ana").remove(0);
    let href = format!("cac:user/{ANA}");
    assert!(mention_link(&ana).contains(&href));
    assert_eq!(mentioned_user(&href), Some(ANA));

    assert_eq!(mentioned_user("https://cac.guz-studio.dev"), None);
    assert_eq!(mentioned_user("cac:user/"), None);
    assert_eq!(mentioned_user("cac:user/no-es-un-uuid"), None);
    assert_eq!(mentioned_user(&format!("{href}/más")), None);
    assert_eq!(mentioned_user(&format!("https://x/?a={href}")), None);
    assert_eq!(mentioned_user(&format!("cac:user/{}", "z".repeat(36))), None);
}
