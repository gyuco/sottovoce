//! Eventi di Google Calendar (calendario principale, sola lettura) e tipi
//! comuni a tutti i provider ([`CalendarEvent`], [`Attendee`]).
//!
//! `singleEvents=true`: Google espande le ricorrenze e ogni occorrenza ha il
//! suo `id` stabile, quindi non serve interpretare le RRULE.

use serde::Deserialize;

use crate::error::CalendarError;
use crate::link::{clean_description, find_call_link};
use crate::time::{format_utc, parse_rfc3339};

const EVENTS_URL: &str = "https://www.googleapis.com/calendar/v3/calendars/primary/events";
/// Limite di pagine per sincronizzazione (250 eventi l'una).
const MAX_PAGES: usize = 8;

/// Partecipante dell'evento (non l'utente stesso, niente sale riunioni).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attendee {
    /// Nome mostrato; vuoto se Google non lo conosce.
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarEvent {
    /// ID stabile dell'occorrenza.
    pub id: String,
    pub title: String,
    pub starts_at_ms: i64,
    pub ends_at_ms: i64,
    /// Descrizione senza HTML né blocco standard con i dettagli di accesso.
    pub description: String,
    /// Link della call: Meet (`hangoutLink`/`conferenceData`) oppure Teams o
    /// Zoom trovati in luogo e descrizione.
    pub join_url: Option<String>,
    pub attendees: Vec<Attendee>,
    /// L'utente ha rifiutato l'invito.
    pub declined: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventsPage {
    #[serde(default)]
    items: Vec<RawEvent>,
    next_page_token: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawEvent {
    id: String,
    status: String,
    summary: String,
    description: String,
    location: String,
    start: RawTime,
    end: RawTime,
    attendees: Vec<RawAttendee>,
    hangout_link: Option<String>,
    conference_data: Option<ConferenceData>,
    event_type: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawTime {
    date_time: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawAttendee {
    email: String,
    display_name: String,
    #[serde(rename = "self")]
    is_self: bool,
    resource: bool,
    response_status: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct ConferenceData {
    entry_points: Vec<EntryPoint>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct EntryPoint {
    entry_point_type: String,
    uri: String,
}

/// Eventi con orario (non "tutto il giorno") che si sovrappongono a
/// `[from_ms, to_ms)`, in ordine di inizio. Esclusi annullati, fuori sede,
/// focus time e simili.
pub async fn list_events(
    http: &reqwest::Client,
    access_token: &str,
    from_ms: i64,
    to_ms: i64,
) -> Result<Vec<CalendarEvent>, CalendarError> {
    let (time_min, time_max) = (format_utc(from_ms), format_utc(to_ms));
    let mut events = Vec::new();
    let mut page_token: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let mut query = vec![
            ("timeMin", time_min.as_str()),
            ("timeMax", time_max.as_str()),
            ("singleEvents", "true"),
            ("orderBy", "startTime"),
            ("maxResults", "250"),
        ];
        if let Some(token) = &page_token {
            query.push(("pageToken", token));
        }
        let response = http
            .get(EVENTS_URL)
            .bearer_auth(access_token)
            .query(&query)
            .send()
            .await?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(CalendarError::Revoked);
        }
        let body = response.text().await?;
        if !status.is_success() {
            return Err(CalendarError::Api {
                status: status.as_u16(),
                message: api_message(&body),
            });
        }
        let page: EventsPage =
            serde_json::from_str(&body).map_err(|e| CalendarError::Invalid(e.to_string()))?;
        events.extend(page.items.into_iter().filter_map(convert));
        match page.next_page_token {
            Some(token) => page_token = Some(token),
            None => break,
        }
    }
    Ok(events)
}

/// Messaggio leggibile da `{"error":{"message":…}}` (es. API non abilitata).
fn api_message(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.pointer("/error/message")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| body.chars().take(300).collect())
}

fn convert(raw: RawEvent) -> Option<CalendarEvent> {
    if raw.status == "cancelled"
        || matches!(
            raw.event_type.as_str(),
            "outOfOffice" | "focusTime" | "workingLocation" | "birthday"
        )
    {
        return None;
    }
    let starts_at_ms = parse_rfc3339(raw.start.date_time.as_deref()?)?;
    let ends_at_ms = parse_rfc3339(raw.end.date_time.as_deref()?)?.max(starts_at_ms);
    let join_url = raw
        .hangout_link
        .filter(|l| l.starts_with("https://"))
        .or_else(|| {
            raw.conference_data?
                .entry_points
                .into_iter()
                .find(|e| e.entry_point_type == "video" && e.uri.starts_with("https://"))
                .map(|e| e.uri)
        })
        .or_else(|| find_call_link(&raw.location))
        .or_else(|| find_call_link(&raw.description));
    let declined = raw
        .attendees
        .iter()
        .any(|a| a.is_self && a.response_status == "declined");
    let attendees = raw
        .attendees
        .into_iter()
        .filter(|a| !a.is_self && !a.resource && !a.email.is_empty())
        .map(|a| Attendee {
            name: a.display_name.trim().to_owned(),
            email: a.email.trim().to_owned(),
        })
        .collect();
    Some(CalendarEvent {
        id: raw.id,
        title: raw.summary.trim().to_owned(),
        starts_at_ms,
        ends_at_ms,
        description: clean_description(&raw.description),
        join_url,
        attendees,
        declined,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_google_events() {
        let page: EventsPage = serde_json::from_str(
            r#"{"items":[
              {"id":"a","status":"confirmed","summary":" Sprint review ",
               "start":{"dateTime":"2026-11-12T14:30:00+01:00"},"end":{"dateTime":"2026-11-12T15:00:00+01:00"},
               "hangoutLink":"https://meet.google.com/abc-defg-hij",
               "description":"Agenda<br>budget\n-::~:~::~:~::-\nboilerplate",
               "attendees":[
                 {"email":"me@x.it","self":true,"responseStatus":"accepted"},
                 {"email":"anna@x.it","displayName":"Anna Bianchi"},
                 {"email":"sala@resource.calendar.google.com","resource":true}]},
              {"id":"b","summary":"Teams","start":{"dateTime":"2026-11-13T09:00:00Z"},"end":{"dateTime":"2026-11-13T09:30:00Z"},
               "description":"<a href=\"https://teams.microsoft.com/l/meetup-join/19%3a1\">Join</a>",
               "attendees":[{"email":"me@x.it","self":true,"responseStatus":"declined"}]},
              {"id":"c","summary":"Ferie","start":{"date":"2026-11-14"},"end":{"date":"2026-11-15"}},
              {"id":"d","status":"cancelled","start":{"dateTime":"2026-11-13T09:00:00Z"},"end":{"dateTime":"2026-11-13T09:30:00Z"}},
              {"id":"e","eventType":"focusTime","start":{"dateTime":"2026-11-13T09:00:00Z"},"end":{"dateTime":"2026-11-13T09:30:00Z"}}
            ],"nextPageToken":null}"#,
        )
        .unwrap();
        let events: Vec<_> = page.items.into_iter().filter_map(convert).collect();
        assert_eq!(events.len(), 2);
        let a = &events[0];
        assert_eq!(a.title, "Sprint review");
        assert_eq!(a.ends_at_ms - a.starts_at_ms, 30 * 60_000);
        assert_eq!(
            a.join_url.as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
        assert_eq!(a.description, "Agenda\nbudget");
        assert_eq!(
            a.attendees,
            [Attendee {
                name: "Anna Bianchi".into(),
                email: "anna@x.it".into()
            }]
        );
        assert!(!a.declined);
        let b = &events[1];
        assert!(b.declined);
        assert!(
            b.join_url
                .as_deref()
                .unwrap()
                .starts_with("https://teams.microsoft.com/")
        );
    }

    #[test]
    fn reads_api_error_message() {
        let body = r#"{"error":{"code":403,"message":"Google Calendar API has not been used in project 1"}}"#;
        assert!(api_message(body).starts_with("Google Calendar API"));
        assert_eq!(api_message("boom"), "boom");
    }
}
