//! Two languages, the same two the macOS app ships. The English text is the
//! key, exactly as in the app's `Localizable.strings`, so a sentence reads the
//! same on both surfaces.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    De,
    En,
}

impl Lang {
    /// From `LC_ALL`, `LC_MESSAGES` or `LANG`, in that order — German when the
    /// first one set says so, English otherwise.
    #[must_use]
    pub fn from_environment() -> Self {
        let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|name| std::env::var(name).ok())
            .find(|value| !value.is_empty())
            .unwrap_or_default();
        if locale.to_lowercase().starts_with("de") { Self::De } else { Self::En }
    }

    /// For text that arrives at run time: one of our own sentences is
    /// translated, anything else — a mail server's words — is shown as it came.
    #[must_use]
    pub fn t_owned(self, text: &str) -> String {
        if self == Self::En {
            return text.to_owned();
        }
        GERMAN.iter().find(|(key, _)| *key == text).map_or_else(|| text.to_owned(), |(_, german)| (*german).to_owned())
    }

    #[must_use]
    pub fn t(self, english: &'static str) -> &'static str {
        if self == Self::En {
            return english;
        }
        GERMAN.iter().find(|(key, _)| *key == english).map_or(english, |(_, german)| german)
    }

    /// What a tool call is called in the log.
    #[must_use]
    pub fn event(self, tool: &str) -> String {
        let english = match tool {
            "mail_list_accounts" => "Listed accounts",
            "mail_search" => "Searched mail",
            "mail_refine_search" => "Refined a search",
            "mail_get_message" => "Read a message",
            "mail_get_attachment" => "Downloaded an attachment",
            "mail_get_thread" => "Read a conversation",
            "mail_list_mailboxes" => "Listed mailboxes",
            "mail_mark" => "Marked a message",
            "mail_create_draft" => "Created a draft",
            "mail_prepare_send" => "Prepared a send",
            "mail_prepare_move" => "Prepared a move",
            "mail_prepare_delete" => "Prepared a delete",
            "mail_confirm_action" => "Confirmed an action",
            "mail_get_policy" => "Checked permissions",
            "mail_get_cache_status" => "Checked cache status",
            other => return other.to_owned(),
        };
        self.t(english).to_owned()
    }

    #[must_use]
    pub fn result(self, result: &str) -> &'static str {
        match result {
            "ok" => self.t("Completed"),
            "error" => self.t("Refused"),
            _ => "",
        }
    }

    /// "5 min ago", compact enough for a table cell.
    #[must_use]
    pub fn ago(self, seconds: u64) -> String {
        let (value, unit_de, unit_en) = match seconds {
            0..60 => return self.t("just now").to_owned(),
            60..3600 => (seconds / 60, "Min", "min"),
            3600..86_400 => (seconds / 3600, "Std", "h"),
            _ => (seconds / 86_400, "T", "d"),
        };
        match self {
            Self::De => format!("vor {value} {unit_de}"),
            Self::En => format!("{value} {unit_en} ago"),
        }
    }
}

const GERMAN: &[(&str, &str)] = &[
    ("Menu", "Menü"),
    ("Overview", "Übersicht"),
    ("Mail Accounts", "Mail-Konten"),
    ("MCP Clients", "MCP-Clients"),
    ("Settings", "Einstellungen"),
    ("Updates", "Updates"),
    ("Log", "Protokoll"),
    ("Help", "Hilfe"),
    ("Your mailboxes for AI assistants.", "Deine Postfächer für KI-Assistenten."),
    ("Status", "Status"),
    ("Ready for assistants", "Bereit für Assistenten"),
    ("Not reachable for assistants", "Für Assistenten nicht erreichbar"),
    ("The MCP server is installed.", "Der MCP-Server ist installiert."),
    ("torromail-mcp was not found next to this program or on the PATH.", "torromail-mcp wurde weder neben diesem Programm noch im PATH gefunden."),
    ("No assistant connected", "Kein Assistent verbunden"),
    ("connected", "verbunden"),
    ("Connect one in the MCP Clients section to start using your mail.", "Verbinde einen im Bereich MCP-Clients, um loszulegen."),
    ("Every access is logged.", "Jeder Zugriff wird protokolliert."),
    ("Needs your attention", "Braucht deine Aufmerksamkeit"),
    ("Recent Activity", "Letzte Aktivität"),
    ("No activity yet. It appears here the moment an assistant does something.", "Noch keine Aktivität. Sie erscheint hier, sobald ein Assistent etwas tut."),
    ("No accounts yet.", "Noch keine Konten."),
    ("Press n to add your first account.", "Drücke n, um dein erstes Konto hinzuzufügen."),
    ("Listed accounts", "Konten aufgelistet"),
    ("Searched mail", "E-Mails durchsucht"),
    ("Refined a search", "Suche verfeinert"),
    ("Read a message", "Nachricht gelesen"),
    ("Downloaded an attachment", "Anhang heruntergeladen"),
    ("Read a conversation", "Konversation gelesen"),
    ("Listed mailboxes", "Ordner aufgelistet"),
    ("Marked a message", "Nachricht markiert"),
    ("Created a draft", "Entwurf erstellt"),
    ("Prepared a send", "Senden vorbereitet"),
    ("Prepared a move", "Verschieben vorbereitet"),
    ("Prepared a delete", "Löschen vorbereitet"),
    ("Confirmed an action", "Aktion bestätigt"),
    ("Checked permissions", "Berechtigungen abgefragt"),
    ("Checked cache status", "Cache-Status abgefragt"),
    ("Other client", "Anderer Client"),
    ("Time", "Zeit"),
    ("Client", "Client"),
    ("Account", "Konto"),
    ("Event", "Ereignis"),
    ("Details", "Details"),
    ("Result", "Ergebnis"),
    ("Completed", "Erledigt"),
    ("Refused", "Abgelehnt"),
    ("just now", "gerade eben"),
    ("Last checked", "Zuletzt geprüft"),
    ("Connection", "Verbindung"),
    ("Permissions", "Berechtigungen"),
    ("Folders", "Ordner"),
    ("Search & Cache", "Suche & Cache"),
    ("Connected", "Verbunden"),
    ("Not reachable", "Nicht erreichbar"),
    ("Not tested yet", "Noch nicht getestet"),
    ("No credentials stored yet", "Noch keine Zugangsdaten hinterlegt"),
    ("TorroMail can no longer reach this account.", "TorroMail erreicht dieses Konto nicht mehr."),
    ("Identity", "Identität"),
    ("Sender Name", "Absendername"),
    ("Provider", "Anbieter"),
    ("Login", "Anmeldung"),
    ("Password", "Passwort"),
    ("Username", "Benutzername"),
    ("IMAP Server", "IMAP-Server"),
    ("SMTP Server", "SMTP-Server"),
    ("What connected assistants may do with this account.", "Was verbundene Assistenten mit diesem Konto dürfen."),
    ("Preset", "Voreinstellung"),
    ("Read only", "Nur lesen"),
    ("Read + drafts", "Lesen + Entwürfe"),
    ("Tidy up", "Aufräumen"),
    ("Full access", "Voller Zugriff"),
    ("adjusted", "angepasst"),
    ("Read", "Lesen"),
    ("Read depth", "Lesetiefe"),
    ("No read access", "Kein Lesezugriff"),
    ("Subject and sender", "Betreff und Absender"),
    ("Full message", "Ganze E-Mail"),
    ("Message and attachments", "E-Mail und Anhänge"),
    ("Write", "Schreiben"),
    ("Create drafts", "Entwürfe erstellen"),
    ("Send", "Senden"),
    ("Mark", "Markieren"),
    ("read state, flags", "gelesen, Flaggen"),
    ("Move", "Verschieben"),
    ("Delete", "Löschen"),
    ("to the Trash", "in den Papierkorb"),
    ("Permanent delete", "Endgültig löschen"),
    ("manual only", "nur manuell"),
    ("Per-folder permissions", "Rechte pro Ordner"),
    ("on", "an"),
    ("off", "aus"),
    ("Standard – all folders", "Standard – alle Ordner"),
    ("Standard", "Standard"),
    ("no access", "kein Zugriff"),
    ("read only", "nur lesen"),
    ("write only", "nur schreiben"),
    ("Special folders", "Spezialordner"),
    ("Automatic", "Automatisch"),
    ("Drafts", "Entwürfe"),
    ("Sent", "Gesendet"),
    ("Archive", "Archiv"),
    ("Junk", "Spam"),
    ("Trash", "Papierkorb"),
    ("Cache", "Cache"),
    ("Off", "Aus"),
    ("Subject & sender", "Betreff & Absender"),
    ("Full messages", "Ganze E-Mails"),
    ("Messages & attachments", "E-Mails & Anhänge"),
    ("Limited by the read permission to", "Durch das Leserecht begrenzt auf"),
    ("More caching makes search faster but stores mail content on this machine.", "Mehr Cache beschleunigt die Suche, speichert aber E-Mail-Inhalte auf diesem Rechner."),
    ("Assistants", "Assistenten"),
    ("Not found on this machine", "Auf diesem Rechner nicht gefunden"),
    ("Not connected", "Nicht verbunden"),
    ("Not installed", "Nicht installiert"),
    ("Installed on this machine", "Auf diesem Rechner installiert"),
    ("Set up by hand", "Von Hand einrichten"),
    ("Configured", "Eingerichtet"),
    ("Connection status", "Verbindungsstatus"),
    ("has connected to the server", "hat sich mit dem Server verbunden"),
    ("Has not connected yet", "Hat sich noch nicht verbunden"),
    ("Last connected", "Zuletzt verbunden"),
    ("Config file", "Konfigurationsdatei"),
    ("Written into the client's configuration", "In die Konfiguration des Clients eingetragen"),
    ("Not written into the client's configuration yet", "Noch nicht in die Konfiguration des Clients eingetragen"),
    ("Configuration", "Konfiguration"),
    ("The snippet carries this client's personal access key — treat it like a password.", "Der Ausschnitt enthält den persönlichen Zugangsschlüssel dieses Clients — behandle ihn wie ein Passwort."),
    ("Nothing to set here yet.", "Hier gibt es noch nichts einzustellen."),
    ("Autostart and notifications arrive with the background service.", "Autostart und Benachrichtigungen kommen mit dem Hintergrunddienst."),
    ("Installed version", "Installierte Version"),
    ("Updates arrive through your package manager or the GitHub releases.", "Updates kommen über deinen Paketmanager oder die GitHub-Releases."),
    ("About TorroMail", "Über TorroMail"),
    ("Version", "Version"),
    ("Data directory", "Datenverzeichnis"),
    ("Documentation", "Dokumentation"),
    ("Report a problem", "Problem melden"),
    ("Every action an assistant takes is recorded in the log — that is the first place to look when something went differently than expected.", "Jede Aktion eines Assistenten steht im Protokoll — dort schaust du zuerst nach, wenn etwas anders lief als erwartet."),
    ("edit", "bearbeiten"),
    ("is installed", "ist installiert"),
    ("Needs the extension", "Braucht die Erweiterung"),
    ("Storage", "Speicher"),
    ("Rebuild", "Neu aufbauen"),
    ("Rebuilding…", "Baue neu auf …"),
    ("The cache was rebuilt.", "Der Cache wurde neu aufgebaut."),
    ("Could not rebuild the cache.", "Der Cache konnte nicht neu aufgebaut werden."),
    ("TorroMail server answers", "TorroMail-Server antwortet"),
    ("tools", "Werkzeuge"),
    ("TorroMail server did not answer", "TorroMail-Server hat nicht geantwortet"),
    ("IMAP encryption", "IMAP-Verschlüsselung"),
    ("SMTP encryption", "SMTP-Verschlüsselung"),
    ("New password", "Neues Passwort"),
    ("Leave the password empty to keep the stored one. Saving checks the connection.", "Lass das Passwort leer, um das gespeicherte zu behalten. Beim Speichern wird die Verbindung geprüft."),
    ("A port is a number between 1 and 65535.", "Ein Port ist eine Zahl zwischen 1 und 65535."),
    ("Nothing to save.", "Nichts zu speichern."),
    ("Saved. The connection works.", "Gespeichert. Die Verbindung funktioniert."),
    ("Saved, but the connection does not work.", "Gespeichert, aber die Verbindung funktioniert nicht."),
    ("Loading folders…", "Ordner werden geladen …"),
    ("Could not load folders. Check the connection and try again.", "Ordner konnten nicht geladen werden. Prüfe die Verbindung und versuche es erneut."),
    ("Off: all folders use the permissions above.", "Aus: Alle Ordner verwenden die Rechte aus dem Reiter Berechtigungen."),
    ("test", "testen"),
    ("remove", "entfernen"),
    ("Remove", "Entfernen:"),
    ("This removes the configuration, the stored password and the local cache. Your mail stays on the server.", "Entfernt die Konfiguration, das gespeicherte Passwort und den lokalen Cache. Deine E-Mails bleiben auf dem Server."),
    ("Account removed. Your mail stays on the server.", "Konto entfernt. Deine E-Mails bleiben auf dem Server."),
    ("Account removed, but not everything could be cleaned up.", "Konto entfernt, aber nicht alles ließ sich aufräumen."),
    ("new account", "neues konto"),
    ("next field", "nächstes feld"),
    ("continue", "weiter"),
    ("server details", "serverdaten"),
    ("ctrl+d", "strg+d"),
    ("New Account", "Neues Konto"),
    ("Email", "E-Mail"),
    ("Done", "Fertig"),
    ("Login for", "Anmeldung für"),
    ("Settings found", "Einstellungen gefunden"),
    ("Looking up the settings for", "Suche die Einstellungen für"),
    ("The one-click sign-in is not available in this version yet. Use the password for this mailbox — that only works if your administrator still allows it.", "Die Ein-Klick-Anmeldung ist in dieser Version noch nicht verfügbar. Nimm das Passwort dieses Postfachs — das funktioniert nur, wenn deine Administration es noch erlaubt."),
    ("IMAP Port", "IMAP-Port"),
    ("SMTP Port", "SMTP-Port"),
    ("App password", "App-Passwort"),
    ("The name and address anything you approve is sent as.", "Der Name und die Adresse, unter der alles rausgeht, was du freigibst."),
    ("TorroMail needs an address to work with.", "TorroMail braucht eine Adresse, mit der es arbeiten kann."),
    ("Enter the server by hand.", "Server von Hand eintragen."),
    ("Nothing could be found for this domain — the server details have to come from you.", "Für diese Domain war nichts zu finden — die Server-Angaben musst du selbst eintragen."),
    ("Your normal password will not work for mail apps. Create an app password and paste it here — it is a password just for TorroMail, and you can revoke it any time.", "Dein normales Passwort funktioniert bei Mail-Programmen nicht. Erstelle ein App-Passwort und füge es hier ein — es gilt nur für TorroMail und du kannst es jederzeit widerrufen."),
    ("The one-click sign-in is not available in this version yet. An app password works just as well and needs two-factor to be on.", "Die Ein-Klick-Anmeldung ist in dieser Version noch nicht verfügbar. Ein App-Passwort funktioniert genauso gut — dafür muss die Zwei-Faktor-Anmeldung aktiv sein."),
    ("What may connected assistants do with this account?", "Was dürfen verbundene Assistenten mit diesem Konto tun?"),
    ("You can change this any time.", "Du kannst das jederzeit ändern."),
    ("Checking the connection…", "Verbindung wird geprüft …"),
    ("Connected as", "Verbunden als"),
    ("Connect an assistant in the MCP Clients section so it can use this account.", "Verbinde im Bereich MCP-Clients einen Assistenten, damit er dieses Konto nutzen kann."),
    ("connect", "verbinden"),
    ("reconnect", "neu verbinden"),
    ("disconnect", "trennen"),
    ("accounts", "konten"),
    ("copy", "kopieren"),
    ("key", "schlüssel"),
    ("Disconnect", "Trennen:"),
    ("(y/n)", "(j/n)"),
    ("Access key missing", "Zugangsschlüssel fehlt"),
    ("The client holds its access key", "Der Client hat seinen Zugangsschlüssel"),
    ("Access key missing — reconnect to fix it", "Zugangsschlüssel fehlt — einmal neu verbinden behebt das"),
    ("Account access", "Kontozugriff"),
    ("All accounts", "Alle Konten"),
    ("Selected accounts", "Ausgewählte Konten"),
    ("Includes accounts added later. Account permissions still apply.", "Auch künftig hinzugefügte Konten. Die Kontoberechtigungen gelten weiterhin."),
    ("New accounts need a separate selection. Account permissions still apply.", "Neue Konten müssen einzeln ausgewählt werden. Die Kontoberechtigungen gelten weiterhin."),
    ("Connected. Restart the assistant to load it.", "Verbunden. Starte den Assistenten neu, um es zu laden."),
    ("Disconnected. Its access key no longer works.", "Getrennt. Der Zugangsschlüssel funktioniert nicht mehr."),
    ("Copied to the clipboard.", "In die Zwischenablage kopiert."),
    ("No clipboard helper found.", "Kein Zwischenablage-Programm gefunden."),
    ("Could not connect.", "Verbinden fehlgeschlagen."),
    ("space", "leertaste"),
    ("ctrl+s", "strg+s"),
    ("toggle", "umschalten"),
    ("save", "speichern"),
    ("back", "zurück"),
    ("Saved.", "Gespeichert."),
    ("Could not save.", "Speichern fehlgeschlagen."),
    ("Unsaved changes", "Ungespeicherte Änderungen"),
    ("Discard unsaved changes? (y/n)", "Ungespeicherte Änderungen verwerfen? (j/n)"),
    ("Permanent delete needs the Delete right first.", "„Endgültig löschen“ braucht zuerst das Recht „Löschen“."),
    ("Applies to the whole account.", "Gilt für das ganze Konto."),
    ("select", "wählen"),
    ("tab", "reiter"),
    ("menu", "menü"),
    ("reload", "neu laden"),
    ("quit", "ende"),
    ("sort", "sortieren"),
    ("reverse", "umkehren"),
    ("row", "zeile"),
];
