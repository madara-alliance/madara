use anyhow::Context;

pub fn setup_rayon_threadpool() -> anyhow::Result<()> {
    let available_parallelism = std::thread::available_parallelism()?;
    rayon::ThreadPoolBuilder::new()
        .thread_name(|thread_index| format!("rayon-{}", thread_index))
        .num_threads(available_parallelism.get())
        .build_global()?;
    Ok(())
}

pub fn raise_fdlimit() {
    use fdlimit::Outcome;
    let recommended = 10000;
    match fdlimit::raise_fd_limit() {
        Ok(Outcome::LimitRaised { to, .. }) if to < recommended => {
            tracing::warn!(
                    "The file-descriptor limit for the current process is {to}, which is lower than the recommended {recommended}."
                );
        }
        Ok(Outcome::LimitRaised { to, .. }) => {
            tracing::debug!("File-descriptor limit was raised to {to}.");
        }
        Err(error) => {
            tracing::warn!(
                "Error while trying to raise the file-descriptor limit for the process: {error:#}. The recommended file-descriptor limit is {recommended}."
            );
        }
        Ok(Outcome::Unsupported) => {
            tracing::debug!("Unsupported platform for raising file-descriptor limit.");
        }
    }
}

/// Returns a random Pokémon name.
pub async fn get_random_pokemon_name() -> anyhow::Result<String> {
    use rand::{seq::SliceRandom, thread_rng};
    let res = reqwest::get("https://pokeapi.co/api/v2/pokemon/?limit=1000").await?;
    let body = res.text().await?;
    let json: serde_json::Value = serde_json::from_str(&body)?;

    let pokemon_array = json["results"].as_array().context("Getting result from returned json")?;
    let mut rng = thread_rng();
    let random_pokemon = pokemon_array.choose(&mut rng).context("Choosing a name")?;

    Ok(random_pokemon["name"].as_str().context("Getting name from pokemon object")?.to_string())
}
