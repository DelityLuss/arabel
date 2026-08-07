//! Dictée vocale : capture du micro en local, transcription par une API
//! compatible OpenAI ou par un modèle Whisper local (whisper.cpp).
//!
//! Pourquoi la capture est ICI et pas dans le webview : `getUserMedia` dans
//! WKWebView (macOS) demande un Info.plist ad hoc, a un historique de
//! double-prompt et de promesses qui ne résolvent jamais (wry#1195,
//! tauri#11951), et notre build est signé ad-hoc — chaque rebuild peut
//! redemander l'autorisation. cpal parle directement à CoreAudio / WASAPI /
//! ALSA, et nous rend du PCM : exactement ce que veut Whisper.
//!
//! Le flux : `voice_start` ouvre le micro dans un thread dédié (le `Stream`
//! cpal n'est pas `Send`), `voice_stop` le ferme, ramène les échantillons,
//! les ré-échantillonne en 16 kHz mono et les transcrit. `voice_cancel`
//! jette tout.

use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::FromSample;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

/// Fréquence attendue par Whisper — locale comme API : 16 kHz, mono.
const WHISPER_RATE: u32 = 16_000;
/// Clé du coffre (store.rs) où vit la clé d'API : elle ne retraverse jamais
/// l'IPC une fois posée, c'est Rust qui la relit au moment de transcrire.
const VOICE_KEY: &str = "voice:api-key";
/// Garde-fou : 10 min d'audio au taux du device. Un `voice_start` oublié
/// (fenêtre fermée, crash du front) ne doit pas manger la RAM indéfiniment.
const MAX_SECONDS: usize = 600;

// ─── état ────────────────────────────────────────────────────────────────────

/// Enregistrement en cours. Un seul à la fois : dicter dans deux panneaux
/// simultanément n'a pas de sens et doublerait la charge micro.
pub struct Rec {
    stop: Sender<()>,
    audio: Receiver<Take>,
}
struct Take {
    samples: Vec<f32>,
    rate: u32,
}
#[derive(Default)]
pub struct VoiceState(pub Mutex<Option<Rec>>);

/// Contexte whisper.cpp gardé chaud entre deux dictées : charger un modèle
/// large coûte plusieurs secondes, on ne le refait que si le chemin change.
#[cfg(feature = "local-whisper")]
#[derive(Default)]
pub struct WhisperCache(pub Mutex<Option<(PathBuf, Arc<whisper_rs::WhisperContext>)>>);

// ─── réglages venus du front ─────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceCfg {
    /// "api" | "local"
    pub backend: String,
    /// Racine d'une API compatible OpenAI, p.ex. `https://api.groq.com/openai/v1`.
    #[serde(default)]
    pub api_base: String,
    #[serde(default)]
    pub api_model: String,
    /// Code ISO ("fr", "en") ; vide = détection automatique.
    #[serde(default)]
    pub language: String,
    /// Amorce donnée au modèle : sert surtout à fixer le vocabulaire (noms de
    /// commandes, jargon) et l'orthographe attendue.
    #[serde(default)]
    pub prompt: String,
    /// Chemin du modèle GGML pour le backend local.
    #[serde(default)]
    #[cfg_attr(not(feature = "local-whisper"), allow(dead_code))] // build API-only : personne ne le lit
    pub model_path: String,
}

#[derive(Serialize, Clone)]
struct DlProgress {
    received: u64,
    total: u64,
}

// ─── capture ─────────────────────────────────────────────────────────────────

/// Où l'utilisateur va rouvrir le robinet du micro. macOS pose la question une
/// fois (TCC) ; Windows a DEUX interrupteurs — l'accès micro global et celui des
/// applications de bureau — et refuse en silence si le second est coupé.
fn permission_hint() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        " — System Settings › Privacy & Security › Microphone"
    }
    #[cfg(target_os = "windows")]
    {
        " — Settings › Privacy & security › Microphone (including “Let desktop apps access your microphone”)"
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        ""
    }
}

/// Nom lisible d'un périphérique. cpal 0.18 le range dans une `DeviceDescription`,
/// dont la lecture peut échouer (device débranché entre l'énumération et l'appel).
fn device_name(d: &cpal::Device) -> Option<String> {
    d.description().ok().map(|desc| desc.name().to_string())
}

#[tauri::command]
pub fn voice_devices() -> Result<Vec<String>, String> {
    let host = cpal::default_host();
    let mut names: Vec<String> = host
        .input_devices()
        .map_err(|e| e.to_string())?
        .filter_map(|d| device_name(&d))
        .collect();
    // WASAPI comme ALSA rendent volontiers le même micro plusieurs fois (deux
    // interfaces, deux plugins) : dedup ne mord que sur des voisins, d'où le tri.
    names.sort();
    names.dedup();
    Ok(names)
}

/// Commande `async` à dessein : la première dictée déclenche la demande
/// d'autorisation micro de macOS, et l'ouverture du device ne rend la main
/// qu'une fois l'utilisateur décidé. En commande synchrone, Tauri exécuterait
/// ça sur le thread principal — donc l'UI (et la boîte de dialogue elle-même)
/// figée le temps de répondre.
#[tauri::command]
pub async fn voice_start(
    app: AppHandle,
    state: tauri::State<'_, VoiceState>,
    device: Option<String>,
) -> Result<(), String> {
    {
        let slot = state.0.lock().map_err(|_| "voice state poisoned")?;
        if slot.is_some() {
            return Err("already recording".into());
        }
    } // verrou relâché : rien ne doit être tenu à travers un await
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let (audio_tx, audio_rx) = mpsc::channel::<Take>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    std::thread::spawn(move || capture(app, device, stop_rx, audio_tx, ready_tx));
    // On attend le verdict d'ouverture du flux : un micro refusé ou absent doit
    // remonter à l'appel, sinon l'utilisateur croit dicter dans le vide.
    let ready = tokio::task::spawn_blocking(move || ready_rx.recv_timeout(Duration::from_secs(120)))
        .await
        .map_err(|e| e.to_string())?;
    match ready {
        Ok(Ok(())) => {
            let mut slot = state.0.lock().map_err(|_| "voice state poisoned")?;
            if slot.is_some() {
                // deux démarrages concurrents : on referme le nôtre plutôt que
                // d'écraser l'autre et de laisser un thread de capture orphelin.
                let _ = stop_tx.send(());
                return Err("already recording".into());
            }
            *slot = Some(Rec { stop: stop_tx, audio: audio_rx });
            Ok(())
        }
        Ok(Err(e)) => Err(e),
        Err(_) => Err(format!("the microphone did not start{}", permission_hint())),
    }
}

/// Thread propriétaire du flux cpal : `cpal::Stream` n'est pas `Send`, il doit
/// naître et mourir sur le même thread. On y reste bloqué jusqu'au signal
/// d'arrêt, puis on ferme le flux AVANT de lire le tampon (plus de callback en
/// vol = pas de demi-trame).
fn capture(
    app: AppHandle,
    want: Option<String>,
    stop_rx: Receiver<()>,
    audio_tx: Sender<Take>,
    ready_tx: Sender<Result<(), String>>,
) {
    // flux ouvert + tampon partagé avec le callback audio + fréquence du device
    type Opened = (cpal::Stream, Arc<Mutex<Vec<f32>>>, u32);
    let built = (|| -> Result<Opened, String> {
        let host = cpal::default_host();
        let device = match &want {
            Some(name) => host
                .input_devices()
                .map_err(|e| e.to_string())?
                .find(|d| device_name(d).as_ref() == Some(name))
                .ok_or_else(|| format!("microphone “{name}” not found"))?,
            None => host
                .default_input_device()
                .ok_or("no microphone found on this machine")?,
        };
        let supported = device
            .default_input_config()
            .map_err(|e| format!("cannot read the microphone config: {e}"))?;
        let rate = supported.sample_rate();
        let channels = supported.channels() as usize;
        let cfg: cpal::StreamConfig = supported.config();
        let buf = Arc::new(Mutex::new(Vec::<f32>::new()));
        let cap = rate as usize * channels.max(1) * MAX_SECONDS;
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => open::<f32>(&device, &cfg, channels, cap, &buf, &app),
            cpal::SampleFormat::I16 => open::<i16>(&device, &cfg, channels, cap, &buf, &app),
            cpal::SampleFormat::U16 => open::<u16>(&device, &cfg, channels, cap, &buf, &app),
            cpal::SampleFormat::I32 => open::<i32>(&device, &cfg, channels, cap, &buf, &app),
            cpal::SampleFormat::I8 => open::<i8>(&device, &cfg, channels, cap, &buf, &app),
            cpal::SampleFormat::U8 => open::<u8>(&device, &cfg, channels, cap, &buf, &app),
            other => Err(format!("unsupported sample format: {other:?}")),
        }?;
        stream.play().map_err(|e| format!("cannot start the microphone: {e}"))?;
        Ok((stream, buf, rate))
    })();

    match built {
        Err(e) => {
            let _ = ready_tx.send(Err(e));
        }
        Ok((stream, buf, rate)) => {
            let _ = ready_tx.send(Ok(()));
            let _ = stop_rx.recv(); // bloque jusqu'à voice_stop / voice_cancel
            drop(stream);
            let samples = std::mem::take(&mut *buf.lock().unwrap_or_else(|e| e.into_inner()));
            let _ = audio_tx.send(Take { samples, rate });
        }
    }
}

/// Ouvre le flux pour un format d'échantillon donné : downmix mono immédiat
/// (Whisper est mono) et niveau crête émis vers l'UI pour le vumètre.
fn open<T>(
    device: &cpal::Device,
    cfg: &cpal::StreamConfig,
    channels: usize,
    cap: usize,
    buf: &Arc<Mutex<Vec<f32>>>,
    app: &AppHandle,
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    let buf = Arc::clone(buf);
    let app = app.clone();
    let mut last = Instant::now();
    let ch = channels.max(1);
    device
        .build_input_stream::<T, _, _>(
            *cfg,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                let mut peak = 0f32;
                {
                    let mut g = match buf.lock() {
                        Ok(g) => g,
                        Err(e) => e.into_inner(),
                    };
                    for frame in data.chunks(ch) {
                        let sum: f32 = frame.iter().map(|s| f32::from_sample_(*s)).sum();
                        let v = sum / frame.len() as f32;
                        peak = peak.max(v.abs());
                        if g.len() < cap {
                            g.push(v);
                        }
                    }
                }
                // ~15 émissions/s : de quoi animer un vumètre sans inonder l'IPC
                // depuis le thread audio (qui doit rester le plus court possible).
                if last.elapsed() >= Duration::from_millis(66) {
                    last = Instant::now();
                    let _ = app.emit("voice-level", peak.min(1.0));
                }
            },
            |e| eprintln!("[arabel] microphone stream error: {e}"),
            None,
        )
        .map_err(|e| format!("cannot open the microphone: {e}{}", permission_hint()))
}

/// Arrête l'enregistrement et rend l'audio capté, en 16 kHz mono.
fn take(state: &tauri::State<'_, VoiceState>) -> Result<Vec<f32>, String> {
    let rec = state
        .0
        .lock()
        .map_err(|_| "voice state poisoned")?
        .take()
        .ok_or("not recording")?;
    let _ = rec.stop.send(());
    let got = rec
        .audio
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| "the microphone did not stop cleanly")?;
    Ok(resample(&got.samples, got.rate, WHISPER_RATE))
}

#[tauri::command]
pub fn voice_cancel(state: tauri::State<'_, VoiceState>) {
    if let Ok(mut slot) = state.0.lock() {
        if let Some(rec) = slot.take() {
            let _ = rec.stop.send(());
        }
    }
}

/// Ré-échantillonnage vers 16 kHz. À la baisse (le cas courant : 44,1 / 48 kHz)
/// on moyenne la fenêtre source — un filtre « boîte » qui coupe l'essentiel du
/// repliement, contrairement au plus proche voisin. À la hausse (8 kHz), une
/// interpolation linéaire suffit. La parole encaisse très bien les deux.
fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if input.is_empty() || from == 0 {
        return Vec::new();
    }
    if from == to {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let out_len = (input.len() as f64 / ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let start = i as f64 * ratio;
        let end = start + ratio;
        if ratio >= 1.0 {
            let a = start as usize;
            let b = (end.ceil() as usize).min(input.len());
            if a >= b {
                break;
            }
            out.push(input[a..b].iter().sum::<f32>() / (b - a) as f32);
        } else {
            let a = start.floor() as usize;
            let frac = (start - a as f64) as f32;
            let s0 = input[a.min(input.len() - 1)];
            let s1 = input[(a + 1).min(input.len() - 1)];
            out.push(s0 + (s1 - s0) * frac);
        }
    }
    out
}

// ─── transcription ───────────────────────────────────────────────────────────

#[tauri::command]
pub async fn voice_stop(
    app: AppHandle,
    state: tauri::State<'_, VoiceState>,
    cfg: VoiceCfg,
) -> Result<String, String> {
    let pcm = take(&state)?;
    // < 0,3 s : un aller-retour de clic, pas une phrase. On évite l'appel réseau
    // (facturé au minimum de 10 s chez certains fournisseurs) et le « Merci. »
    // que Whisper hallucine sur du silence.
    if pcm.len() < (WHISPER_RATE as usize) / 3 {
        return Ok(String::new());
    }
    match cfg.backend.as_str() {
        "local" => transcribe_local(&app, pcm, cfg).await,
        _ => transcribe_api(pcm, cfg).await,
    }
}

/// API compatible OpenAI (`POST {base}/audio/transcriptions`, multipart). Un
/// seul chemin de code sert OpenAI, Groq, ou un whisper.cpp/faster-whisper
/// auto-hébergé : seule la base URL change.
async fn transcribe_api(pcm: Vec<f32>, cfg: VoiceCfg) -> Result<String, String> {
    let base = cfg.api_base.trim().trim_end_matches('/').to_string();
    if base.is_empty() {
        return Err("no API endpoint configured (Settings → Voice)".into());
    }
    let key = crate::store::passphrase_get(VOICE_KEY).unwrap_or_default();
    let wav = wav16(&pcm);
    let mut form = reqwest::multipart::Form::new()
        .part(
            "file",
            reqwest::multipart::Part::bytes(wav)
                .file_name("audio.wav")
                .mime_str("audio/wav")
                .map_err(|e| e.to_string())?,
        )
        .text("model", cfg.api_model.clone())
        .text("response_format", "json");
    if !cfg.language.trim().is_empty() {
        form = form.text("language", cfg.language.trim().to_string());
    }
    if !cfg.prompt.trim().is_empty() {
        form = form.text("prompt", cfg.prompt.trim().to_string());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;
    let mut req = client.post(format!("{base}/audio/transcriptions")).multipart(form);
    if !key.is_empty() {
        req = req.bearer_auth(key);
    }
    let res = req.send().await.map_err(|e| format!("transcription request failed: {e}"))?;
    let status = res.status();
    let body = res.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        // Le corps d'erreur porte le vrai motif (clé invalide, modèle inconnu,
        // quota) : on le remonte tel quel plutôt qu'un « 401 » opaque.
        return Err(format!("{status}: {}", body.trim()));
    }
    let parsed: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    Ok(parsed
        .get("text")
        .and_then(|t| t.as_str())
        .unwrap_or_default()
        .trim()
        .to_string())
}

#[cfg(feature = "local-whisper")]
async fn transcribe_local(app: &AppHandle, pcm: Vec<f32>, cfg: VoiceCfg) -> Result<String, String> {
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    let path = PathBuf::from(cfg.model_path.trim());
    if cfg.model_path.trim().is_empty() || !path.is_file() {
        return Err("no local model — download one in Settings → Voice".into());
    }
    let cache = app.state::<WhisperCache>();
    let cached = {
        let g = cache.0.lock().map_err(|_| "whisper cache poisoned")?;
        g.as_ref().filter(|(p, _)| *p == path).map(|(_, c)| Arc::clone(c))
    };
    let ctx = match cached {
        Some(c) => c,
        None => {
            let p = path.clone();
            // Charger un modèle bloque le thread plusieurs secondes (large-v3 =
            // ~1 Go à mapper) : hors du runtime async.
            let fresh = tokio::task::spawn_blocking(move || {
                WhisperContext::new_with_params(&p, WhisperContextParameters::default())
                    .map_err(|e| format!("cannot load the model: {e}"))
            })
            .await
            .map_err(|e| e.to_string())??;
            let fresh = Arc::new(fresh);
            if let Ok(mut g) = cache.0.lock() {
                *g = Some((path.clone(), Arc::clone(&fresh)));
            }
            fresh
        }
    };

    let lang = cfg.language.trim().to_string();
    let prompt = cfg.prompt.trim().to_string();
    tokio::task::spawn_blocking(move || {
        let mut state = ctx.create_state().map_err(|e| e.to_string())?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        // whisper.cpp écrit sur stdout par défaut : on le fait taire, sinon il
        // pollue la sortie de l'app.
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_translate(false); // dicter en français doit rendre du français
        params.set_no_timestamps(true);
        params.set_suppress_blank(true);
        // Un thread par cœur, moins un, pour que l'UI reste réactive pendant
        // l'inférence (elle sature volontiers toutes les unités dispo).
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        params.set_n_threads((threads.saturating_sub(1).max(1)) as i32);
        if !lang.is_empty() {
            params.set_language(Some(&lang));
        }
        if !prompt.is_empty() {
            params.set_initial_prompt(&prompt);
        }
        state.full(params, &pcm).map_err(|e| format!("transcription failed: {e}"))?;
        let mut out = String::new();
        for i in 0..state.full_n_segments() {
            if let Some(seg) = state.get_segment(i) {
                if let Ok(t) = seg.to_str_lossy() {
                    out.push_str(&t);
                }
            }
        }
        Ok(out.trim().to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(not(feature = "local-whisper"))]
async fn transcribe_local(_app: &AppHandle, _pcm: Vec<f32>, _cfg: VoiceCfg) -> Result<String, String> {
    Err("this build has no local Whisper — rebuild with the `local-whisper` feature, or use the API backend".into())
}

/// Le backend local est-il compilé dans ce binaire ? L'UI grise l'onglet sinon,
/// au lieu de proposer un réglage qui échouera à l'usage.
#[tauri::command]
pub fn voice_local_available() -> bool {
    cfg!(feature = "local-whisper")
}

/// WAV PCM 16 bits mono 16 kHz. 44 octets d'en-tête, pas de dépendance.
fn wav16(pcm: &[f32]) -> Vec<u8> {
    let data_len = (pcm.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // taille du bloc fmt
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM entier
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&WHISPER_RATE.to_le_bytes());
    out.extend_from_slice(&(WHISPER_RATE * 2).to_le_bytes()); // octets/s
    out.extend_from_slice(&2u16.to_le_bytes()); // alignement de bloc
    out.extend_from_slice(&16u16.to_le_bytes()); // bits par échantillon
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in pcm {
        // clamp avant conversion : un échantillon > 1.0 (gain d'entrée poussé)
        // bouclerait par débordement et claquerait dans l'audio.
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes());
    }
    out
}

// ─── modèles locaux ──────────────────────────────────────────────────────────

/// Les modèles vont dans les données LOCALES, pas dans la config : un GGML pèse
/// de 150 Mo à 1,5 Go, et sur Windows `app_config_dir` est `%APPDATA%` (Roaming)
/// — un profil itinérant se traînerait le modèle à chaque ouverture de session.
/// Sur Linux ça tombe dans `~/.local/share`, sa vraie place ; sur macOS c'est le
/// même Application Support qu'avant.
fn models_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_local_data_dir()
        .map_err(|e| e.to_string())?
        .join("models");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

#[tauri::command]
pub fn voice_models(app: AppHandle) -> Result<Vec<(String, u64)>, String> {
    let dir = models_dir(&app)?;
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten() {
        let meta = match entry.metadata() {
            Ok(m) if m.is_file() => m,
            _ => continue,
        };
        if let Some(name) = entry.path().to_str() {
            out.push((name.to_string(), meta.len()));
        }
    }
    out.sort();
    Ok(out)
}

/// Télécharge un modèle GGML. Écrit dans un `.part` renommé à la fin : une
/// coupure réseau laisse un fichier partiel identifiable, jamais un modèle
/// tronqué que whisper.cpp chargerait à moitié.
#[tauri::command]
pub async fn voice_model_download(app: AppHandle, url: String, name: String) -> Result<String, String> {
    use futures_util::StreamExt;

    // Pas de séparateur de chemin dans le nom : il vient d'une liste, mais rien
    // n'empêche un jour de le rendre libre — on ne veut pas écrire hors du dossier.
    let safe = name.replace(['/', '\\'], "_");
    if safe.is_empty() {
        return Err("invalid model name".into());
    }
    let dir = models_dir(&app)?;
    let dest = dir.join(&safe);
    let part = dir.join(format!("{safe}.part"));

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60 * 60))
        .build()
        .map_err(|e| e.to_string())?;
    let res = client.get(&url).send().await.map_err(|e| format!("download failed: {e}"))?;
    if !res.status().is_success() {
        return Err(format!("download failed: {}", res.status()));
    }
    let total = res.content_length().unwrap_or(0);
    let mut file = std::fs::File::create(&part).map_err(|e| e.to_string())?;
    let mut received: u64 = 0;
    let mut last = Instant::now();
    let mut stream = res.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("download interrupted: {e}"))?;
        file.write_all(&chunk).map_err(|e| e.to_string())?;
        received += chunk.len() as u64;
        if last.elapsed() >= Duration::from_millis(200) {
            last = Instant::now();
            let _ = app.emit("voice-download", DlProgress { received, total });
        }
    }
    file.flush().map_err(|e| e.to_string())?;
    drop(file);
    std::fs::rename(&part, &dest).map_err(|e| e.to_string())?;
    let _ = app.emit("voice-download", DlProgress { received, total: received });
    dest.to_str().map(str::to_string).ok_or_else(|| "invalid path".into())
}

/// Supprimer un modèle exige de le RELÂCHER d'abord : whisper.cpp mappe le
/// fichier en mémoire et Windows refuse d'effacer un fichier encore ouvert (là
/// où Unix l'accepte sans broncher). On vide donc le cache s'il pointe dessus.
#[tauri::command]
pub fn voice_model_delete(app: AppHandle, path: String) -> Result<(), String> {
    #[cfg(feature = "local-whisper")]
    {
        let cache = app.state::<WhisperCache>();
        // `let` et pas `if let` : le verrou doit être une liaison nommée, sinon
        // il vit jusqu'à la fin de l'instruction et emprunte `cache` plus
        // longtemps que celui-ci n'existe.
        let mut g = match cache.0.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        if g.as_ref().is_some_and(|(p, _)| p == &PathBuf::from(&path)) {
            *g = None; // dernière référence au contexte → le fichier est refermé
        }
        drop(g);
    }
    #[cfg(not(feature = "local-whisper"))]
    let _ = &app;
    std::fs::remove_file(path).map_err(|e| e.to_string())
}

// ─── clé d'API (coffre chiffré) ──────────────────────────────────────────────

#[tauri::command]
pub fn voice_key_set(key: String) -> Result<(), String> {
    if key.trim().is_empty() {
        crate::store::passphrase_delete(VOICE_KEY.to_string())
    } else {
        crate::store::passphrase_set(VOICE_KEY.to_string(), key)
    }
}

/// L'UI ne demande jamais la clé en retour — juste si elle est posée, pour
/// afficher « saved » plutôt que de la ré-exposer dans un champ.
#[tauri::command]
pub fn voice_key_present() -> bool {
    crate::store::passphrase_get(VOICE_KEY).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downsamples_by_averaging() {
        // 48 kHz → 16 kHz : 3 échantillons source pour 1 en sortie, moyennés.
        let src: Vec<f32> = vec![0.0, 0.3, 0.6, 1.0, 1.0, 1.0];
        let out = resample(&src, 48_000, 16_000);
        assert_eq!(out.len(), 2);
        assert!((out[0] - 0.3).abs() < 1e-6);
        assert!((out[1] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn passthrough_when_already_16k() {
        let src = vec![0.1, -0.2, 0.3];
        assert_eq!(resample(&src, 16_000, 16_000), src);
        assert!(resample(&[], 48_000, 16_000).is_empty());
    }

    #[test]
    fn upsamples_without_panicking_on_the_last_sample() {
        let out = resample(&[0.0, 1.0], 8_000, 16_000);
        assert_eq!(out.len(), 4);
        assert!(out.iter().all(|v| (-1.0..=1.0).contains(v)));
    }

    #[test]
    fn wav_header_is_well_formed() {
        let wav = wav16(&[0.0, 1.0, -1.0]);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(wav.len(), 44 + 6);
        // taille RIFF = tout sauf les 8 premiers octets
        assert_eq!(u32::from_le_bytes(wav[4..8].try_into().unwrap()), 42);
        // saturation : +1.0 → i16::MAX, -1.0 → -i16::MAX (pas de bouclage)
        assert_eq!(i16::from_le_bytes(wav[46..48].try_into().unwrap()), i16::MAX);
        assert_eq!(i16::from_le_bytes(wav[48..50].try_into().unwrap()), -i16::MAX);
    }

    #[test]
    fn clamps_out_of_range_samples() {
        let wav = wav16(&[3.0, -3.0]);
        assert_eq!(i16::from_le_bytes(wav[44..46].try_into().unwrap()), i16::MAX);
        assert_eq!(i16::from_le_bytes(wav[46..48].try_into().unwrap()), -i16::MAX);
    }
}
