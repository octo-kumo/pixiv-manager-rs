use super::common;

fn parse_mode(mode: &str) -> pixivj_rs::model::FilterMode {
    match mode.to_lowercase().as_str() {
        "day" => pixivj_rs::model::FilterMode::Day,
        "day_male" | "daymale" => pixivj_rs::model::FilterMode::DayMale,
        "day_female" | "dayfemale" => pixivj_rs::model::FilterMode::DayFemale,
        "week" => pixivj_rs::model::FilterMode::Week,
        "week_original" | "weekoriginal" => pixivj_rs::model::FilterMode::WeekOriginal,
        "week_rookie" | "weekrookie" => pixivj_rs::model::FilterMode::WeekRookie,
        "month" => pixivj_rs::model::FilterMode::Month,
        "month_rookie" | "monthrookie" => pixivj_rs::model::FilterMode::MonthRookie,
        "day_r18" | "dayr18" => pixivj_rs::model::FilterMode::DayR18,
        "week_r18" | "weekr18" => pixivj_rs::model::FilterMode::WeekR18,
        "week_r18g" | "weekr18g" => pixivj_rs::model::FilterMode::WeekR18g,
        "day_male_r18" | "daymaler18" => pixivj_rs::model::FilterMode::DayMaleR18,
        "day_female_r18" | "dayfemaler18" => pixivj_rs::model::FilterMode::DayFemaleR18,
        "day_manga" | "daymanga" => pixivj_rs::model::FilterMode::DayManga,
        _ => {
            eprintln!(
                "Unknown ranking mode '{}'. Available: day, week, month, day_male, day_female, \
                 week_original, week_rookie, month_rookie, day_r18, week_r18, week_r18g, \
                 day_male_r18, day_female_r18, day_manga",
                mode
            );
            std::process::exit(1);
        }
    }
}

pub async fn run(mode: Option<String>, json: bool) {
    let client = common::client_or_exit().await;

    let filter = pixivj_rs::model::RankedIllustsFilter {
        mode: mode.as_deref().map(parse_mode),
        filter: None,
        date: None,
        offset: None,
    };

    match client.get_ranked_illusts(&filter).await {
        Ok(result) => {
            common::print_illusts(&result.illusts, json);
        }
        Err(e) => {
            eprintln!("Failed to fetch ranking: {}", e);
            std::process::exit(1);
        }
    }
}
