use pixivj_rs::model::Illustration;

/// Format illustrations as an aligned-column table string.
pub fn format_illust_table(illusts: &[Illustration]) -> String {
    if illusts.is_empty() {
        return "No results.".to_string();
    }

    // Find max widths for each column
    let mut max_id = 2;
    let mut max_title = 5;
    let mut max_author = 6;
    let mut max_bm = 9;
    let mut max_views = 5;
    let mut max_date = 4;

    for i in illusts {
        max_id = max_id.max(i.id.to_string().len());
        let title = i.title.as_deref().unwrap_or("-");
        max_title = max_title.max(title.len().min(60));
        let author = i.user.as_ref().map(|u| u.name.as_str()).unwrap_or("-");
        max_author = max_author.max(author.len());
        let bm = i.total_bookmarks.unwrap_or(0);
        max_bm = max_bm.max(bm.to_string().len());
        let views = i.total_view.unwrap_or(0);
        max_views = max_views.max(views.to_string().len());
        let date = i
            .create_date
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "-".to_string());
        max_date = max_date.max(date.len());
    }

    // Build header
    let header = format!(
        "{:>id$}  {:<title$}  {:<author$}  {:>bm$}  {:>views$}  {:<date$}",
        "ID",
        "TITLE",
        "AUTHOR",
        "BOOKMARKS",
        "VIEWS",
        "DATE",
        id = max_id,
        title = max_title,
        author = max_author,
        bm = max_bm,
        views = max_views,
        date = max_date,
    );

    let sep = "-".repeat(header.len());
    let mut out = vec![header, sep];

    for i in illusts {
        let title = i.title.as_deref().unwrap_or("-");
        let title_trunc = if title.len() > max_title {
            &title[..max_title - 1]
        } else {
            title
        };
        let author = i.user.as_ref().map(|u| u.name.as_str()).unwrap_or("-");
        let bm = i.total_bookmarks.unwrap_or(0);
        let views = i.total_view.unwrap_or(0);
        let date = i
            .create_date
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "-".to_string());

        out.push(format!(
            "{:>id$}  {:<title$}  {:<author$}  {:>bm$}  {:>views$}  {:<date$}",
            i.id,
            title_trunc,
            author,
            bm,
            views,
            date,
            id = max_id,
            title = max_title,
            author = max_author,
            bm = max_bm,
            views = max_views,
            date = max_date,
        ));
    }

    out.join("\n")
}

/// Format illustrations as pretty JSON.
pub fn format_illust_json(illusts: &[Illustration]) -> String {
    serde_json::to_string_pretty(illusts).unwrap_or_else(|e| format!("JSON error: {}", e))
}
