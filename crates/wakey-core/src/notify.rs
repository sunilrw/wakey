use std::process::Command;

/// Sends a non-blocking OS desktop notification.
pub fn send_desktop_notification(title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    {
        let esc_title = title.replace('"', "\\\"");
        let esc_body = body.replace('"', "\\\"");
        let script = format!("display notification \"{esc_body}\" with title \"{esc_title}\"");
        let _ = Command::new("osascript").args(["-e", &script]).spawn();
    }

    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("notify-send").args([title, body]).spawn();
    }

    #[cfg(target_os = "windows")]
    {
        let esc_title = title.replace('\'', "''");
        let esc_body = body.replace('\'', "''");
        let ps = format!(
            "[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null; \
             $xml = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02); \
             $nodes = $xml.GetElementsByTagName('text'); \
             $nodes.Item(0).AppendChild($xml.CreateTextNode('{esc_title}')) | Out-Null; \
             $nodes.Item(1).AppendChild($xml.CreateTextNode('{esc_body}')) | Out-Null; \
             $toast = [Windows.UI.Notifications.ToastNotification]::new($xml); \
             [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('wakey').Show($toast);"
        );
        let _ = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &ps])
            .spawn();
    }
}
