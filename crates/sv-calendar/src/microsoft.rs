//! Eventi del calendario Outlook / Microsoft 365 via Microsoft Graph
//! (calendario predefinito, sola lettura).
//!
//! `calendarView` espande le ricorrenze: ogni occorrenza ha il suo `id`.
//! Con `Prefer: outlook.timezone="UTC"` gli orari arrivano in UTC senza
//! offset, con `outlook.body-content-type="text"` la descrizione è testo.

use serde::Deserialize;

use crate::error::CalendarError;
use crate::google::{Attendee, CalendarEvent};
use crate::link::{clean_description, find_call_link};
use crate::time::{format_utc, parse_rfc3339};

const CALENDAR_VIEW_URL: &str = "https://graph.microsoft.com/v1.0/me/calendarView";
const SELECT: &str = "id,subject,start,end,isAllDay,isCancelled,onlineMeeting,onlineMeetingUrl,\
                      location,body,attendees,responseStatus";
/// Limite di pagine per sincronizzazione (100 eventi l'una).
const MAX_PAGES: usize = 15;

#[derive(Deserialize)]
struct EventsPage {
    #[serde(default)]
    value: Vec<RawEvent>,
    #[serde(rename = "@odata.nextLink")]
    next_link: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawEvent {
    id: String,
    subject: Option<String>,
    start: RawTime,
    end: RawTime,
    is_all_day: bool,
    is_cancelled: bool,
    online_meeting: Option<OnlineMeeting>,
    online_meeting_url: Option<String>,
    location: Option<Location>,
    body: Option<Body>,
    attendees: Vec<RawAttendee>,
    response_status: Option<ResponseStatus>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawTime {
    date_time: String,
    time_zone: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct OnlineMeeting {
    join_url: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct Location {
    display_name: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Body {
    content: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawAttendee {
    #[serde(rename = "type")]
    kind: String,
    email_address: EmailAddress,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct EmailAddress {
    name: String,
    address: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ResponseStatus {
    response: String,
}

/// Eventi con orario (non "tutto il giorno") che si sovrappongono a
/// `[from_ms, to_ms)`, in ordine di inizio. `self_email`: l'account collegato,
/// tolto dai partecipanti.
pub async fn list_events(
    http: &reqwest::Client,
    access_token: &str,
    from_ms: i64,
    to_ms: i64,
    self_email: Option<&str>,
) -> Result<Vec<CalendarEvent>, CalendarError> {
    let mut url = url::Url::parse_with_params(
        CALENDAR_VIEW_URL,
        &[
            ("startDateTime", format_utc(from_ms).as_str()),
            ("endDateTime", format_utc(to_ms).as_str()),
            ("$select", SELECT),
            ("$orderby", "start/dateTime"),
            ("$top", "100"),
        ],
    )
    .map_err(|e| CalendarError::Invalid(e.to_string()))?
    .to_string();
    let mut events = Vec::new();
    for _ in 0..MAX_PAGES {
        let response = http
            .get(&url)
            .bearer_auth(access_token)
            .header(
                "Prefer",
                "outlook.timezone=\"UTC\", outlook.body-content-type=\"text\"",
            )
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
        events.extend(
            page.value
                .into_iter()
                .filter_map(|e| convert(e, self_email)),
        );
        // Il link successivo deve restare su Graph: il token non va altrove.
        match page.next_link {
            Some(next) if next.starts_with("https://graph.microsoft.com/") => url = next,
            _ => break,
        }
    }
    Ok(events)
}

/// Messaggio leggibile da `{"error":{"message":…}}`.
fn api_message(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.pointer("/error/message")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| body.chars().take(300).collect())
}

/// Orario di Graph (`2026-11-12T13:30:00.0000000`, fuso a parte) → ms epoch.
/// Chiediamo UTC; un altro fuso (non dovrebbe accadere) scarta l'evento.
fn parse_time(time: &RawTime) -> Option<i64> {
    if !time.time_zone.is_empty() && !time.time_zone.eq_ignore_ascii_case("UTC") {
        return None;
    }
    let value = time.date_time.trim();
    if value.ends_with('Z') || value.contains('+') {
        parse_rfc3339(value)
    } else {
        parse_rfc3339(&format!("{value}Z"))
    }
}

fn convert(raw: RawEvent, self_email: Option<&str>) -> Option<CalendarEvent> {
    if raw.is_cancelled || raw.is_all_day {
        return None;
    }
    let starts_at_ms = parse_time(&raw.start)?;
    let ends_at_ms = parse_time(&raw.end)?.max(starts_at_ms);
    let location = raw.location.map(|l| l.display_name).unwrap_or_default();
    let body = raw.body.map(|b| b.content).unwrap_or_default();
    let join_url = raw
        .online_meeting
        .and_then(|m| m.join_url)
        .or(raw.online_meeting_url)
        .filter(|l| l.starts_with("https://"))
        .or_else(|| find_call_link(&location))
        .or_else(|| find_call_link(&body));
    let declined = raw
        .response_status
        .is_some_and(|r| r.response == "declined");
    let attendees = raw
        .attendees
        .into_iter()
        .filter(|a| a.kind != "resource" && !a.email_address.address.is_empty())
        .filter(|a| self_email.is_none_or(|me| !a.email_address.address.eq_ignore_ascii_case(me)))
        .map(|a| {
            let name = a.email_address.name.trim();
            let email = a.email_address.address.trim();
            Attendee {
                // Senza nome Graph ripete l'indirizzo.
                name: if name.eq_ignore_ascii_case(email) {
                    String::new()
                } else {
                    name.to_owned()
                },
                email: email.to_owned(),
            }
        })
        .collect();
    Some(CalendarEvent {
        id: raw.id,
        title: raw.subject.unwrap_or_default().trim().to_owned(),
        starts_at_ms,
        ends_at_ms,
        description: clean_description(&body),
        join_url,
        attendees,
        declined,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_graph_events() {
        let page: EventsPage = serde_json::from_str(
            r#"{"value":[
              {"id":"AAMk1","subject":"Sprint review",
               "start":{"dateTime":"2026-11-12T13:30:00.0000000","timeZone":"UTC"},
               "end":{"dateTime":"2026-11-12T14:00:00.0000000","timeZone":"UTC"},
               "isAllDay":false,"isCancelled":false,
               "onlineMeeting":{"joinUrl":"https://teams.microsoft.com/l/meetup-join/19%3ameeting_X%40thread.v2/0"},
               "body":{"contentType":"text","content":"Agenda: budget\n________________________________________________________________________________\nRiunione di Microsoft Teams"},
               "responseStatus":{"response":"accepted"},
               "attendees":[
                 {"type":"required","emailAddress":{"name":"Anna Bianchi","address":"anna@contoso.com"}},
                 {"type":"required","emailAddress":{"name":"me@contoso.com","address":"Me@contoso.com"}},
                 {"type":"optional","emailAddress":{"name":"marco@contoso.com","address":"marco@contoso.com"}},
                 {"type":"resource","emailAddress":{"name":"Sala 1","address":"sala1@contoso.com"}}]},
              {"id":"AAMk2","subject":"Zoom esterno",
               "start":{"dateTime":"2026-11-13T09:00:00.0000000","timeZone":"UTC"},
               "end":{"dateTime":"2026-11-13T09:30:00.0000000","timeZone":"UTC"},
               "location":{"displayName":"https://us02web.zoom.us/j/123"},
               "responseStatus":{"response":"declined"}},
              {"id":"AAMk3","subject":"Ferie","isAllDay":true,
               "start":{"dateTime":"2026-11-14T00:00:00.0000000","timeZone":"UTC"},
               "end":{"dateTime":"2026-11-15T00:00:00.0000000","timeZone":"UTC"}},
              {"id":"AAMk4","isCancelled":true,
               "start":{"dateTime":"2026-11-13T09:00:00.0000000","timeZone":"UTC"},
               "end":{"dateTime":"2026-11-13T09:30:00.0000000","timeZone":"UTC"}}
            ]}"#,
        )
        .unwrap();
        let events: Vec<_> = page
            .value
            .into_iter()
            .filter_map(|e| convert(e, Some("me@contoso.com")))
            .collect();
        assert_eq!(events.len(), 2);
        let a = &events[0];
        assert_eq!(format_utc(a.starts_at_ms), "2026-11-12T13:30:00Z");
        assert_eq!(a.ends_at_ms - a.starts_at_ms, 30 * 60_000);
        assert!(
            a.join_url
                .as_deref()
                .unwrap()
                .starts_with("https://teams.microsoft.com/l/meetup-join/")
        );
        assert_eq!(a.description, "Agenda: budget");
        let people: Vec<_> = a
            .attendees
            .iter()
            .map(|p| (p.name.as_str(), p.email.as_str()))
            .collect();
        assert_eq!(
            people,
            [
                ("Anna Bianchi", "anna@contoso.com"),
                ("", "marco@contoso.com")
            ]
        );
        let b = &events[1];
        assert!(b.declined);
        assert_eq!(b.join_url.as_deref(), Some("https://us02web.zoom.us/j/123"));
    }

    #[test]
    fn rejects_non_utc_times() {
        let local = RawTime {
            date_time: "2026-11-12T13:30:00".into(),
            time_zone: "W. Europe Standard Time".into(),
        };
        assert_eq!(parse_time(&local), None);
    }
}
