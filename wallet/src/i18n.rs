//! Internationalization (i18n) — multi-language message catalog for the wallet.
//!
//! Provides locale detection, string interpolation with named placeholders,
//! and a message catalog covering all wallet operations. Supports dynamic
//! locale switching without restart.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ──────────────────────────── Types ──────────────────────────────────────

/// Supported locales.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Locale {
    En, // English (default)
    Es, // Spanish
    Fr, // French
    De, // German
    Ja, // Japanese
    Zh, // Chinese (Simplified)
    Ko, // Korean
    Hi, // Hindi
    Ar, // Arabic
    Pt, // Portuguese
    Ru, // Russian
}

impl Locale {
    /// Parse a locale string (e.g. "en", "es", "fr-FR", "zh-CN").
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Locale> {
        let lower = s.to_lowercase();
        let tag = lower.split(['-', '_']).next().unwrap_or("");
        match tag {
            "en" => Some(Locale::En),
            "es" => Some(Locale::Es),
            "fr" => Some(Locale::Fr),
            "de" => Some(Locale::De),
            "ja" => Some(Locale::Ja),
            "zh" => Some(Locale::Zh),
            "ko" => Some(Locale::Ko),
            "hi" => Some(Locale::Hi),
            "ar" => Some(Locale::Ar),
            "pt" => Some(Locale::Pt),
            "ru" => Some(Locale::Ru),
            _ => None,
        }
    }

    /// All supported locales.
    pub fn all() -> &'static [Locale] {
        &[
            Locale::En,
            Locale::Es,
            Locale::Fr,
            Locale::De,
            Locale::Ja,
            Locale::Zh,
            Locale::Ko,
            Locale::Hi,
            Locale::Ar,
            Locale::Pt,
            Locale::Ru,
        ]
    }

    /// Language name in its own script.
    pub fn native_name(&self) -> &'static str {
        match self {
            Locale::En => "English",
            Locale::Es => "Español",
            Locale::Fr => "Français",
            Locale::De => "Deutsch",
            Locale::Ja => "日本語",
            Locale::Zh => "中文",
            Locale::Ko => "한국어",
            Locale::Hi => "हिन्दी",
            Locale::Ar => "العربية",
            Locale::Pt => "Português",
            Locale::Ru => "Русский",
        }
    }

    /// ISO 639-1 code.
    pub fn code(&self) -> &'static str {
        match self {
            Locale::En => "en",
            Locale::Es => "es",
            Locale::Fr => "fr",
            Locale::De => "de",
            Locale::Ja => "ja",
            Locale::Zh => "zh",
            Locale::Ko => "ko",
            Locale::Hi => "hi",
            Locale::Ar => "ar",
            Locale::Pt => "pt",
            Locale::Ru => "ru",
        }
    }
}

impl std::fmt::Display for Locale {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.code())
    }
}

/// Message keys used throughout the wallet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MsgKey {
    // ── General ──
    Welcome,
    Error,
    Success,
    Confirm,
    Cancel,
    Yes,
    No,
    Loading,
    Done,

    // ── Account ──
    AccountCreated,
    AccountSwitched,
    AccountBalance,
    AccountNotFound,
    NoActiveAccount,

    // ── Transfer ──
    TransferSent,
    TransferConfirmed,
    TransferFailed,
    InsufficientBalance,

    // ── Energy ──
    EnergyLow,
    EnergyCritical,
    EnergyRefreshed,
    EnergyForecast,
    ObjectEvaporated,

    // ── Staking ──
    Staked,
    Unstaked,
    RewardsClaimed,

    // ── Governance ──
    VoteCast,
    ProposalCreated,

    // ── NFT / Token ──
    NftMinted,
    NftTransferred,
    TokenDeployed,
    TokenTransferred,

    // ── Security ──
    PasswordPrompt,
    BackupCreated,
    BackupRestored,
    SpendingLimitExceeded,
    AddressBlocked,
    MultisigApproved,

    // ── Network ──
    NodeConnected,
    NodeDisconnected,
    SyncComplete,

    // ── Bridge ──
    BridgeInitiated,
    BridgeCompleted,

    // ── Misc ──
    FaucetReceived,
    GasEstimate,
    SimulationResult,
    HookFired,
    SessionExpired,
}

// ──────────────────────────── Catalog ────────────────────────────────────

/// The i18n engine: holds all translations and current locale.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct I18n {
    locale: Locale,
    catalog: HashMap<Locale, HashMap<MsgKey, String>>,
}

/// Error type for i18n operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum I18nError {
    #[error("unsupported locale: {0}")]
    UnsupportedLocale(String),
    #[error("missing message key: {0:?} for locale {1}")]
    MissingKey(MsgKey, Locale),
    #[error("invalid placeholder: {0}")]
    InvalidPlaceholder(String),
}

impl I18n {
    /// Create with default English catalog.
    pub fn new() -> Self {
        let mut catalog = HashMap::new();
        catalog.insert(Locale::En, Self::english());
        catalog.insert(Locale::Es, Self::spanish());
        catalog.insert(Locale::Fr, Self::french());
        catalog.insert(Locale::De, Self::german());
        catalog.insert(Locale::Hi, Self::hindi());
        catalog.insert(Locale::Ja, Self::japanese());
        catalog.insert(Locale::Zh, Self::chinese());
        catalog.insert(Locale::Ko, Self::korean());
        catalog.insert(Locale::Pt, Self::portuguese());
        catalog.insert(Locale::Ru, Self::russian());
        catalog.insert(Locale::Ar, Self::arabic());
        Self {
            locale: Locale::En,
            catalog,
        }
    }

    /// Create with a specific locale.
    pub fn with_locale(locale: Locale) -> Self {
        let mut i18n = Self::new();
        i18n.locale = locale;
        i18n
    }

    /// Detect locale from environment variables (LANG, LC_ALL, LANGUAGE).
    pub fn from_env() -> Self {
        let locale = std::env::var("LC_ALL")
            .or_else(|_| std::env::var("LANG"))
            .or_else(|_| std::env::var("LANGUAGE"))
            .ok()
            .and_then(|v| Locale::from_str(&v))
            .unwrap_or(Locale::En);
        Self::with_locale(locale)
    }

    /// Current locale.
    pub fn locale(&self) -> Locale {
        self.locale
    }

    /// Switch locale.
    pub fn set_locale(&mut self, locale: Locale) {
        self.locale = locale;
    }

    /// Set locale from string, returns error if unsupported.
    pub fn set_locale_str(&mut self, s: &str) -> Result<(), I18nError> {
        let locale =
            Locale::from_str(s).ok_or_else(|| I18nError::UnsupportedLocale(s.to_string()))?;
        self.locale = locale;
        Ok(())
    }

    /// Get a raw message (no interpolation).
    pub fn get(&self, key: MsgKey) -> &str {
        self.catalog
            .get(&self.locale)
            .and_then(|m| m.get(&key))
            .or_else(|| self.catalog.get(&Locale::En).and_then(|m| m.get(&key)))
            .map(|s| s.as_str())
            .unwrap_or("???")
    }

    /// Get a message with named placeholder interpolation.
    /// Placeholders use `{name}` syntax.
    pub fn format(&self, key: MsgKey, vars: &[(&str, &str)]) -> String {
        let template = self.get(key);
        let mut result = template.to_string();
        for (name, value) in vars {
            result = result.replace(&format!("{{{}}}", name), value);
        }
        result
    }

    /// List all supported locales with native names.
    pub fn supported_locales(&self) -> Vec<(Locale, &'static str)> {
        Locale::all()
            .iter()
            .map(|l| (*l, l.native_name()))
            .collect()
    }

    /// Check if a locale has a full translation catalog.
    pub fn is_complete(&self, locale: Locale) -> bool {
        let en_keys = self.catalog.get(&Locale::En).map(|m| m.len()).unwrap_or(0);
        self.catalog.get(&locale).map(|m| m.len()).unwrap_or(0) >= en_keys
    }

    /// Get translation completeness percentage for a locale.
    pub fn completeness(&self, locale: Locale) -> f64 {
        let en_keys = self.catalog.get(&Locale::En).map(|m| m.len()).unwrap_or(1) as f64;
        let locale_keys = self.catalog.get(&locale).map(|m| m.len()).unwrap_or(0) as f64;
        (locale_keys / en_keys * 100.0).min(100.0)
    }

    /// Add or override a translation for a specific locale and key.
    pub fn set_translation(&mut self, locale: Locale, key: MsgKey, value: String) {
        self.catalog.entry(locale).or_default().insert(key, value);
    }

    // ── Built-in catalogs ──

    fn english() -> HashMap<MsgKey, String> {
        let mut m = HashMap::new();
        m.insert(MsgKey::Welcome, "Welcome to EvaporChain Wallet".into());
        m.insert(MsgKey::Error, "Error: {message}".into());
        m.insert(MsgKey::Success, "Success!".into());
        m.insert(MsgKey::Confirm, "Are you sure?".into());
        m.insert(MsgKey::Cancel, "Cancelled.".into());
        m.insert(MsgKey::Yes, "Yes".into());
        m.insert(MsgKey::No, "No".into());
        m.insert(MsgKey::Loading, "Loading...".into());
        m.insert(MsgKey::Done, "Done.".into());
        m.insert(
            MsgKey::AccountCreated,
            "Account '{name}' created at {address}".into(),
        );
        m.insert(
            MsgKey::AccountSwitched,
            "Switched to account '{name}'".into(),
        );
        m.insert(MsgKey::AccountBalance, "Balance: {amount} EVAP".into());
        m.insert(MsgKey::AccountNotFound, "Account '{name}' not found".into());
        m.insert(
            MsgKey::NoActiveAccount,
            "No active account. Run: wallet account create <name>".into(),
        );
        m.insert(MsgKey::TransferSent, "Sent {amount} EVAP to {to}".into());
        m.insert(
            MsgKey::TransferConfirmed,
            "Transfer confirmed in block {block}".into(),
        );
        m.insert(MsgKey::TransferFailed, "Transfer failed: {reason}".into());
        m.insert(
            MsgKey::InsufficientBalance,
            "Insufficient balance: have {have}, need {need}".into(),
        );
        m.insert(
            MsgKey::EnergyLow,
            "Warning: Object {id} energy at {pct}%".into(),
        );
        m.insert(
            MsgKey::EnergyCritical,
            "CRITICAL: Object {id} energy at {pct}% — evaporation imminent!".into(),
        );
        m.insert(
            MsgKey::EnergyRefreshed,
            "Object {id} refreshed with {energy} energy".into(),
        );
        m.insert(
            MsgKey::EnergyForecast,
            "Object {id}: {pct}% energy, ~{epochs} epochs until evaporation".into(),
        );
        m.insert(
            MsgKey::ObjectEvaporated,
            "Object {id} has evaporated (ghost state)".into(),
        );
        m.insert(MsgKey::Staked, "Staked {amount} EVAP in pool {pool}".into());
        m.insert(
            MsgKey::Unstaked,
            "Unstaked {amount} EVAP from pool {pool}".into(),
        );
        m.insert(
            MsgKey::RewardsClaimed,
            "Claimed {amount} EVAP rewards from pool {pool}".into(),
        );
        m.insert(
            MsgKey::VoteCast,
            "Vote cast on proposal #{id}: {option}".into(),
        );
        m.insert(
            MsgKey::ProposalCreated,
            "Proposal #{id} created: {title}".into(),
        );
        m.insert(MsgKey::NftMinted, "NFT '{name}' minted (ID: {id})".into());
        m.insert(
            MsgKey::NftTransferred,
            "NFT {id} transferred to {to}".into(),
        );
        m.insert(
            MsgKey::TokenDeployed,
            "Token '{symbol}' deployed (ID: {id})".into(),
        );
        m.insert(
            MsgKey::TokenTransferred,
            "Transferred {amount} {symbol} to {to}".into(),
        );
        m.insert(MsgKey::PasswordPrompt, "Enter password: ".into());
        m.insert(MsgKey::BackupCreated, "Backup saved to {file}".into());
        m.insert(MsgKey::BackupRestored, "Backup restored from {file}".into());
        m.insert(
            MsgKey::SpendingLimitExceeded,
            "Spending limit exceeded: {amount} > {limit}".into(),
        );
        m.insert(
            MsgKey::AddressBlocked,
            "Address {address} is blocked".into(),
        );
        m.insert(
            MsgKey::MultisigApproved,
            "Proposal {id} approved ({count}/{threshold})".into(),
        );
        m.insert(MsgKey::NodeConnected, "Connected to {url}".into());
        m.insert(MsgKey::NodeDisconnected, "Disconnected from node".into());
        m.insert(
            MsgKey::SyncComplete,
            "Sync complete at block {height}".into(),
        );
        m.insert(
            MsgKey::BridgeInitiated,
            "Bridge transfer initiated: {amount} {token} to {chain}".into(),
        );
        m.insert(
            MsgKey::BridgeCompleted,
            "Bridge transfer completed: {id}".into(),
        );
        m.insert(
            MsgKey::FaucetReceived,
            "Received testnet tokens at {address}".into(),
        );
        m.insert(
            MsgKey::GasEstimate,
            "Estimated gas: {gas} (fee: {fee} EVAP)".into(),
        );
        m.insert(
            MsgKey::SimulationResult,
            "Simulation: balance {change}, fee {fee}".into(),
        );
        m.insert(MsgKey::HookFired, "Hook '{name}' fired on {event}".into());
        m.insert(MsgKey::SessionExpired, "Session '{id}' expired".into());
        m
    }

    fn spanish() -> HashMap<MsgKey, String> {
        let mut m = HashMap::new();
        m.insert(MsgKey::Welcome, "Bienvenido a EvaporChain Wallet".into());
        m.insert(MsgKey::Error, "Error: {message}".into());
        m.insert(MsgKey::Success, "¡Éxito!".into());
        m.insert(MsgKey::Confirm, "¿Está seguro?".into());
        m.insert(MsgKey::Cancel, "Cancelado.".into());
        m.insert(MsgKey::Yes, "Sí".into());
        m.insert(MsgKey::No, "No".into());
        m.insert(MsgKey::Loading, "Cargando...".into());
        m.insert(MsgKey::Done, "Listo.".into());
        m.insert(
            MsgKey::AccountCreated,
            "Cuenta '{name}' creada en {address}".into(),
        );
        m.insert(MsgKey::AccountSwitched, "Cambiado a cuenta '{name}'".into());
        m.insert(MsgKey::AccountBalance, "Saldo: {amount} EVAP".into());
        m.insert(
            MsgKey::AccountNotFound,
            "Cuenta '{name}' no encontrada".into(),
        );
        m.insert(
            MsgKey::NoActiveAccount,
            "Sin cuenta activa. Ejecute: wallet account create <nombre>".into(),
        );
        m.insert(MsgKey::TransferSent, "Enviados {amount} EVAP a {to}".into());
        m.insert(
            MsgKey::TransferConfirmed,
            "Transferencia confirmada en bloque {block}".into(),
        );
        m.insert(
            MsgKey::TransferFailed,
            "Transferencia fallida: {reason}".into(),
        );
        m.insert(
            MsgKey::InsufficientBalance,
            "Saldo insuficiente: tiene {have}, necesita {need}".into(),
        );
        m.insert(
            MsgKey::EnergyLow,
            "Aviso: Objeto {id} energía al {pct}%".into(),
        );
        m.insert(
            MsgKey::EnergyCritical,
            "CRÍTICO: Objeto {id} energía al {pct}% — ¡evaporación inminente!".into(),
        );
        m.insert(
            MsgKey::EnergyRefreshed,
            "Objeto {id} recargado con {energy} energía".into(),
        );
        m.insert(
            MsgKey::EnergyForecast,
            "Objeto {id}: {pct}% energía, ~{epochs} épocas hasta evaporación".into(),
        );
        m.insert(
            MsgKey::ObjectEvaporated,
            "Objeto {id} se ha evaporado (estado fantasma)".into(),
        );
        m.insert(
            MsgKey::Staked,
            "{amount} EVAP apostados en pool {pool}".into(),
        );
        m.insert(
            MsgKey::Unstaked,
            "{amount} EVAP retirados del pool {pool}".into(),
        );
        m.insert(
            MsgKey::RewardsClaimed,
            "Reclamados {amount} EVAP de recompensas del pool {pool}".into(),
        );
        m.insert(
            MsgKey::VoteCast,
            "Voto emitido en propuesta #{id}: {option}".into(),
        );
        m.insert(
            MsgKey::ProposalCreated,
            "Propuesta #{id} creada: {title}".into(),
        );
        m.insert(MsgKey::NftMinted, "NFT '{name}' acuñado (ID: {id})".into());
        m.insert(MsgKey::NftTransferred, "NFT {id} transferido a {to}".into());
        m.insert(
            MsgKey::TokenDeployed,
            "Token '{symbol}' desplegado (ID: {id})".into(),
        );
        m.insert(
            MsgKey::TokenTransferred,
            "Transferidos {amount} {symbol} a {to}".into(),
        );
        m.insert(MsgKey::PasswordPrompt, "Introduzca contraseña: ".into());
        m.insert(
            MsgKey::BackupCreated,
            "Copia de seguridad guardada en {file}".into(),
        );
        m.insert(
            MsgKey::BackupRestored,
            "Copia de seguridad restaurada de {file}".into(),
        );
        m.insert(
            MsgKey::SpendingLimitExceeded,
            "Límite de gasto excedido: {amount} > {limit}".into(),
        );
        m.insert(
            MsgKey::AddressBlocked,
            "Dirección {address} bloqueada".into(),
        );
        m.insert(
            MsgKey::MultisigApproved,
            "Propuesta {id} aprobada ({count}/{threshold})".into(),
        );
        m.insert(MsgKey::NodeConnected, "Conectado a {url}".into());
        m.insert(MsgKey::NodeDisconnected, "Desconectado del nodo".into());
        m.insert(
            MsgKey::SyncComplete,
            "Sincronización completa en bloque {height}".into(),
        );
        m.insert(
            MsgKey::BridgeInitiated,
            "Transferencia puente iniciada: {amount} {token} a {chain}".into(),
        );
        m.insert(
            MsgKey::BridgeCompleted,
            "Transferencia puente completada: {id}".into(),
        );
        m.insert(
            MsgKey::FaucetReceived,
            "Tokens de prueba recibidos en {address}".into(),
        );
        m.insert(
            MsgKey::GasEstimate,
            "Gas estimado: {gas} (tarifa: {fee} EVAP)".into(),
        );
        m.insert(
            MsgKey::SimulationResult,
            "Simulación: saldo {change}, tarifa {fee}".into(),
        );
        m.insert(
            MsgKey::HookFired,
            "Hook '{name}' activado en {event}".into(),
        );
        m.insert(MsgKey::SessionExpired, "Sesión '{id}' expirada".into());
        m
    }

    fn french() -> HashMap<MsgKey, String> {
        let mut m = HashMap::new();
        m.insert(MsgKey::Welcome, "Bienvenue sur EvaporChain Wallet".into());
        m.insert(MsgKey::Error, "Erreur : {message}".into());
        m.insert(MsgKey::Success, "Succès !".into());
        m.insert(MsgKey::Confirm, "Êtes-vous sûr ?".into());
        m.insert(MsgKey::Cancel, "Annulé.".into());
        m.insert(MsgKey::Yes, "Oui".into());
        m.insert(MsgKey::No, "Non".into());
        m.insert(MsgKey::Loading, "Chargement...".into());
        m.insert(MsgKey::Done, "Terminé.".into());
        m.insert(
            MsgKey::AccountCreated,
            "Compte '{name}' créé à {address}".into(),
        );
        m.insert(
            MsgKey::AccountSwitched,
            "Basculé vers le compte '{name}'".into(),
        );
        m.insert(MsgKey::AccountBalance, "Solde : {amount} EVAP".into());
        m.insert(
            MsgKey::AccountNotFound,
            "Compte '{name}' introuvable".into(),
        );
        m.insert(
            MsgKey::NoActiveAccount,
            "Aucun compte actif. Lancez : wallet account create <nom>".into(),
        );
        m.insert(MsgKey::TransferSent, "{amount} EVAP envoyés à {to}".into());
        m.insert(
            MsgKey::TransferConfirmed,
            "Transfert confirmé au bloc {block}".into(),
        );
        m.insert(MsgKey::TransferFailed, "Transfert échoué : {reason}".into());
        m.insert(
            MsgKey::InsufficientBalance,
            "Solde insuffisant : {have} disponible, {need} requis".into(),
        );
        m.insert(
            MsgKey::EnergyLow,
            "Attention : Objet {id} énergie à {pct}%".into(),
        );
        m.insert(
            MsgKey::EnergyCritical,
            "CRITIQUE : Objet {id} énergie à {pct}% — évaporation imminente !".into(),
        );
        m.insert(
            MsgKey::EnergyRefreshed,
            "Objet {id} rechargé avec {energy} énergie".into(),
        );
        m.insert(
            MsgKey::EnergyForecast,
            "Objet {id} : {pct}% énergie, ~{epochs} époques avant évaporation".into(),
        );
        m.insert(
            MsgKey::ObjectEvaporated,
            "L'objet {id} s'est évaporé (état fantôme)".into(),
        );
        m.insert(
            MsgKey::Staked,
            "{amount} EVAP misés dans le pool {pool}".into(),
        );
        m.insert(
            MsgKey::Unstaked,
            "{amount} EVAP retirés du pool {pool}".into(),
        );
        m.insert(
            MsgKey::RewardsClaimed,
            "{amount} EVAP de récompenses réclamés du pool {pool}".into(),
        );
        m.insert(
            MsgKey::VoteCast,
            "Vote émis sur la proposition #{id} : {option}".into(),
        );
        m.insert(
            MsgKey::ProposalCreated,
            "Proposition #{id} créée : {title}".into(),
        );
        m.insert(MsgKey::NftMinted, "NFT '{name}' frappé (ID : {id})".into());
        m.insert(MsgKey::NftTransferred, "NFT {id} transféré à {to}".into());
        m.insert(
            MsgKey::TokenDeployed,
            "Token '{symbol}' déployé (ID : {id})".into(),
        );
        m.insert(
            MsgKey::TokenTransferred,
            "{amount} {symbol} transférés à {to}".into(),
        );
        m.insert(MsgKey::PasswordPrompt, "Entrez le mot de passe : ".into());
        m.insert(
            MsgKey::BackupCreated,
            "Sauvegarde enregistrée dans {file}".into(),
        );
        m.insert(
            MsgKey::BackupRestored,
            "Sauvegarde restaurée depuis {file}".into(),
        );
        m.insert(
            MsgKey::SpendingLimitExceeded,
            "Limite de dépense dépassée : {amount} > {limit}".into(),
        );
        m.insert(MsgKey::AddressBlocked, "Adresse {address} bloquée".into());
        m.insert(
            MsgKey::MultisigApproved,
            "Proposition {id} approuvée ({count}/{threshold})".into(),
        );
        m.insert(MsgKey::NodeConnected, "Connecté à {url}".into());
        m.insert(MsgKey::NodeDisconnected, "Déconnecté du nœud".into());
        m.insert(
            MsgKey::SyncComplete,
            "Synchronisation terminée au bloc {height}".into(),
        );
        m.insert(
            MsgKey::BridgeInitiated,
            "Transfert pont initié : {amount} {token} vers {chain}".into(),
        );
        m.insert(
            MsgKey::BridgeCompleted,
            "Transfert pont terminé : {id}".into(),
        );
        m.insert(
            MsgKey::FaucetReceived,
            "Tokens de test reçus à {address}".into(),
        );
        m.insert(
            MsgKey::GasEstimate,
            "Gas estimé : {gas} (frais : {fee} EVAP)".into(),
        );
        m.insert(
            MsgKey::SimulationResult,
            "Simulation : solde {change}, frais {fee}".into(),
        );
        m.insert(
            MsgKey::HookFired,
            "Hook '{name}' déclenché sur {event}".into(),
        );
        m.insert(MsgKey::SessionExpired, "Session '{id}' expirée".into());
        m
    }

    fn german() -> HashMap<MsgKey, String> {
        let mut m = HashMap::new();
        m.insert(MsgKey::Welcome, "Willkommen bei EvaporChain Wallet".into());
        m.insert(MsgKey::Error, "Fehler: {message}".into());
        m.insert(MsgKey::Success, "Erfolg!".into());
        m.insert(MsgKey::Confirm, "Sind Sie sicher?".into());
        m.insert(MsgKey::Cancel, "Abgebrochen.".into());
        m.insert(MsgKey::Yes, "Ja".into());
        m.insert(MsgKey::No, "Nein".into());
        m.insert(MsgKey::Loading, "Laden...".into());
        m.insert(MsgKey::Done, "Fertig.".into());
        m.insert(
            MsgKey::AccountCreated,
            "Konto '{name}' erstellt unter {address}".into(),
        );
        m.insert(
            MsgKey::AccountSwitched,
            "Gewechselt zu Konto '{name}'".into(),
        );
        m.insert(MsgKey::AccountBalance, "Guthaben: {amount} EVAP".into());
        m.insert(
            MsgKey::AccountNotFound,
            "Konto '{name}' nicht gefunden".into(),
        );
        m.insert(
            MsgKey::NoActiveAccount,
            "Kein aktives Konto. Führen Sie aus: wallet account create <name>".into(),
        );
        m.insert(
            MsgKey::TransferSent,
            "{amount} EVAP an {to} gesendet".into(),
        );
        m.insert(
            MsgKey::TransferConfirmed,
            "Überweisung bestätigt in Block {block}".into(),
        );
        m.insert(
            MsgKey::TransferFailed,
            "Überweisung fehlgeschlagen: {reason}".into(),
        );
        m.insert(
            MsgKey::InsufficientBalance,
            "Unzureichendes Guthaben: {have} vorhanden, {need} benötigt".into(),
        );
        m.insert(
            MsgKey::EnergyLow,
            "Warnung: Objekt {id} Energie bei {pct}%".into(),
        );
        m.insert(
            MsgKey::EnergyCritical,
            "KRITISCH: Objekt {id} Energie bei {pct}% — Verdampfung steht bevor!".into(),
        );
        m.insert(
            MsgKey::EnergyRefreshed,
            "Objekt {id} mit {energy} Energie aufgeladen".into(),
        );
        m.insert(
            MsgKey::EnergyForecast,
            "Objekt {id}: {pct}% Energie, ~{epochs} Epochen bis Verdampfung".into(),
        );
        m.insert(
            MsgKey::ObjectEvaporated,
            "Objekt {id} ist verdampft (Geisterzustand)".into(),
        );
        m.insert(
            MsgKey::Staked,
            "{amount} EVAP in Pool {pool} gestaked".into(),
        );
        m.insert(
            MsgKey::Unstaked,
            "{amount} EVAP aus Pool {pool} entstaked".into(),
        );
        m.insert(
            MsgKey::RewardsClaimed,
            "{amount} EVAP Belohnungen aus Pool {pool} beansprucht".into(),
        );
        m.insert(
            MsgKey::VoteCast,
            "Abstimmung für Vorschlag #{id}: {option}".into(),
        );
        m.insert(
            MsgKey::ProposalCreated,
            "Vorschlag #{id} erstellt: {title}".into(),
        );
        m.insert(MsgKey::NftMinted, "NFT '{name}' geprägt (ID: {id})".into());
        m.insert(MsgKey::NftTransferred, "NFT {id} übertragen an {to}".into());
        m.insert(
            MsgKey::TokenDeployed,
            "Token '{symbol}' bereitgestellt (ID: {id})".into(),
        );
        m.insert(
            MsgKey::TokenTransferred,
            "{amount} {symbol} an {to} übertragen".into(),
        );
        m.insert(MsgKey::PasswordPrompt, "Passwort eingeben: ".into());
        m.insert(
            MsgKey::BackupCreated,
            "Sicherung gespeichert in {file}".into(),
        );
        m.insert(
            MsgKey::BackupRestored,
            "Sicherung wiederhergestellt aus {file}".into(),
        );
        m.insert(
            MsgKey::SpendingLimitExceeded,
            "Ausgabenlimit überschritten: {amount} > {limit}".into(),
        );
        m.insert(MsgKey::AddressBlocked, "Adresse {address} gesperrt".into());
        m.insert(
            MsgKey::MultisigApproved,
            "Vorschlag {id} genehmigt ({count}/{threshold})".into(),
        );
        m.insert(MsgKey::NodeConnected, "Verbunden mit {url}".into());
        m.insert(MsgKey::NodeDisconnected, "Vom Knoten getrennt".into());
        m.insert(
            MsgKey::SyncComplete,
            "Synchronisation abgeschlossen bei Block {height}".into(),
        );
        m.insert(
            MsgKey::BridgeInitiated,
            "Bridge-Transfer gestartet: {amount} {token} nach {chain}".into(),
        );
        m.insert(
            MsgKey::BridgeCompleted,
            "Bridge-Transfer abgeschlossen: {id}".into(),
        );
        m.insert(
            MsgKey::FaucetReceived,
            "Testnet-Token empfangen unter {address}".into(),
        );
        m.insert(
            MsgKey::GasEstimate,
            "Geschätztes Gas: {gas} (Gebühr: {fee} EVAP)".into(),
        );
        m.insert(
            MsgKey::SimulationResult,
            "Simulation: Saldo {change}, Gebühr {fee}".into(),
        );
        m.insert(
            MsgKey::HookFired,
            "Hook '{name}' ausgelöst bei {event}".into(),
        );
        m.insert(MsgKey::SessionExpired, "Sitzung '{id}' abgelaufen".into());
        m
    }

    fn hindi() -> HashMap<MsgKey, String> {
        let mut m = HashMap::new();
        m.insert(MsgKey::Welcome, "EvaporChain Wallet में आपका स्वागत है".into());
        m.insert(MsgKey::Error, "त्रुटि: {message}".into());
        m.insert(MsgKey::Success, "सफल!".into());
        m.insert(MsgKey::Confirm, "क्या आप सुनिश्चित हैं?".into());
        m.insert(MsgKey::Cancel, "रद्द किया गया।".into());
        m.insert(MsgKey::Yes, "हाँ".into());
        m.insert(MsgKey::No, "नहीं".into());
        m.insert(MsgKey::Loading, "लोड हो रहा है...".into());
        m.insert(MsgKey::Done, "पूर्ण।".into());
        m.insert(
            MsgKey::AccountCreated,
            "खाता '{name}' बनाया गया: {address}".into(),
        );
        m.insert(MsgKey::AccountSwitched, "खाता '{name}' पर स्विच किया".into());
        m.insert(MsgKey::AccountBalance, "शेष: {amount} EVAP".into());
        m.insert(MsgKey::AccountNotFound, "खाता '{name}' नहीं मिला".into());
        m.insert(
            MsgKey::NoActiveAccount,
            "कोई सक्रिय खाता नहीं। चलाएं: wallet account create <नाम>".into(),
        );
        m.insert(MsgKey::TransferSent, "{to} को {amount} EVAP भेजे गए".into());
        m.insert(
            MsgKey::TransferConfirmed,
            "ब्लॉक {block} में हस्तांतरण की पुष्टि हुई".into(),
        );
        m.insert(MsgKey::TransferFailed, "हस्तांतरण विफल: {reason}".into());
        m.insert(
            MsgKey::InsufficientBalance,
            "अपर्याप्त शेष: {have} उपलब्ध, {need} आवश्यक".into(),
        );
        m.insert(MsgKey::EnergyLow, "चेतावनी: वस्तु {id} ऊर्जा {pct}% पर".into());
        m.insert(
            MsgKey::EnergyCritical,
            "गंभीर: वस्तु {id} ऊर्जा {pct}% पर — वाष्पीकरण आसन्न!".into(),
        );
        m.insert(
            MsgKey::EnergyRefreshed,
            "वस्तु {id} को {energy} ऊर्जा से रिचार्ज किया".into(),
        );
        m.insert(
            MsgKey::EnergyForecast,
            "वस्तु {id}: {pct}% ऊर्जा, वाष्पीकरण तक ~{epochs} युग".into(),
        );
        m.insert(
            MsgKey::ObjectEvaporated,
            "वस्तु {id} वाष्पित हो गई (भूत अवस्था)".into(),
        );
        m.insert(MsgKey::Staked, "पूल {pool} में {amount} EVAP स्टेक किए".into());
        m.insert(
            MsgKey::Unstaked,
            "पूल {pool} से {amount} EVAP अनस्टेक किए".into(),
        );
        m.insert(
            MsgKey::RewardsClaimed,
            "पूल {pool} से {amount} EVAP पुरस्कार प्राप्त किए".into(),
        );
        m.insert(MsgKey::VoteCast, "प्रस्ताव #{id} पर मत: {option}".into());
        m.insert(
            MsgKey::ProposalCreated,
            "प्रस्ताव #{id} बनाया: {title}".into(),
        );
        m.insert(MsgKey::NftMinted, "NFT '{name}' मिंट किया (ID: {id})".into());
        m.insert(MsgKey::NftTransferred, "NFT {id} {to} को हस्तांतरित".into());
        m.insert(
            MsgKey::TokenDeployed,
            "टोकन '{symbol}' तैनात (ID: {id})".into(),
        );
        m.insert(
            MsgKey::TokenTransferred,
            "{to} को {amount} {symbol} हस्तांतरित".into(),
        );
        m.insert(MsgKey::PasswordPrompt, "पासवर्ड दर्ज करें: ".into());
        m.insert(MsgKey::BackupCreated, "बैकअप {file} में सहेजा गया".into());
        m.insert(MsgKey::BackupRestored, "{file} से बैकअप पुनर्स्थापित".into());
        m.insert(
            MsgKey::SpendingLimitExceeded,
            "खर्च सीमा पार: {amount} > {limit}".into(),
        );
        m.insert(MsgKey::AddressBlocked, "पता {address} अवरुद्ध है".into());
        m.insert(
            MsgKey::MultisigApproved,
            "प्रस्ताव {id} स्वीकृत ({count}/{threshold})".into(),
        );
        m.insert(MsgKey::NodeConnected, "{url} से जुड़ा".into());
        m.insert(MsgKey::NodeDisconnected, "नोड से विच्छेद".into());
        m.insert(MsgKey::SyncComplete, "ब्लॉक {height} पर सिंक पूर्ण".into());
        m.insert(
            MsgKey::BridgeInitiated,
            "ब्रिज हस्तांतरण शुरू: {amount} {token} {chain} को".into(),
        );
        m.insert(MsgKey::BridgeCompleted, "ब्रिज हस्तांतरण पूर्ण: {id}".into());
        m.insert(MsgKey::FaucetReceived, "{address} पर टेस्ट टोकन प्राप्त".into());
        m.insert(
            MsgKey::GasEstimate,
            "अनुमानित गैस: {gas} (शुल्क: {fee} EVAP)".into(),
        );
        m.insert(
            MsgKey::SimulationResult,
            "सिमुलेशन: शेष {change}, शुल्क {fee}".into(),
        );
        m.insert(MsgKey::HookFired, "हुक '{name}' {event} पर सक्रिय".into());
        m.insert(MsgKey::SessionExpired, "सत्र '{id}' समाप्त".into());
        m
    }

    fn japanese() -> HashMap<MsgKey, String> {
        let mut m = HashMap::new();
        m.insert(MsgKey::Welcome, "EvaporChain Walletへようこそ".into());
        m.insert(MsgKey::Error, "エラー: {message}".into());
        m.insert(MsgKey::Success, "成功！".into());
        m.insert(MsgKey::Confirm, "よろしいですか？".into());
        m.insert(MsgKey::Cancel, "キャンセルしました。".into());
        m.insert(MsgKey::Yes, "はい".into());
        m.insert(MsgKey::No, "いいえ".into());
        m.insert(MsgKey::Loading, "読み込み中...".into());
        m.insert(MsgKey::Done, "完了。".into());
        m.insert(
            MsgKey::AccountCreated,
            "アカウント'{name}'を{address}に作成しました".into(),
        );
        m.insert(
            MsgKey::AccountSwitched,
            "アカウント'{name}'に切り替えました".into(),
        );
        m.insert(MsgKey::AccountBalance, "残高: {amount} EVAP".into());
        m.insert(
            MsgKey::AccountNotFound,
            "アカウント'{name}'が見つかりません".into(),
        );
        m.insert(
            MsgKey::NoActiveAccount,
            "アクティブなアカウントがありません。実行: wallet account create <名前>".into(),
        );
        m.insert(
            MsgKey::TransferSent,
            "{to}に{amount} EVAPを送信しました".into(),
        );
        m.insert(
            MsgKey::TransferConfirmed,
            "ブロック{block}で送金が確認されました".into(),
        );
        m.insert(MsgKey::TransferFailed, "送金失敗: {reason}".into());
        m.insert(
            MsgKey::InsufficientBalance,
            "残高不足: {have}保有、{need}必要".into(),
        );
        m.insert(
            MsgKey::EnergyLow,
            "警告: オブジェクト{id}のエネルギーが{pct}%です".into(),
        );
        m.insert(
            MsgKey::EnergyCritical,
            "危険: オブジェクト{id}のエネルギーが{pct}% — 蒸発間近！".into(),
        );
        m.insert(
            MsgKey::EnergyRefreshed,
            "オブジェクト{id}に{energy}エネルギーを補充しました".into(),
        );
        m.insert(
            MsgKey::EnergyForecast,
            "オブジェクト{id}: エネルギー{pct}%、蒸発まで約{epochs}エポック".into(),
        );
        m.insert(
            MsgKey::ObjectEvaporated,
            "オブジェクト{id}が蒸発しました（ゴースト状態）".into(),
        );
        m.insert(
            MsgKey::Staked,
            "プール{pool}に{amount} EVAPをステークしました".into(),
        );
        m.insert(
            MsgKey::Unstaked,
            "プール{pool}から{amount} EVAPをアンステークしました".into(),
        );
        m.insert(
            MsgKey::RewardsClaimed,
            "プール{pool}から{amount} EVAPの報酬を受け取りました".into(),
        );
        m.insert(MsgKey::VoteCast, "提案#{id}に投票: {option}".into());
        m.insert(MsgKey::ProposalCreated, "提案#{id}を作成: {title}".into());
        m.insert(
            MsgKey::NftMinted,
            "NFT '{name}'を発行しました (ID: {id})".into(),
        );
        m.insert(
            MsgKey::NftTransferred,
            "NFT {id}を{to}に転送しました".into(),
        );
        m.insert(
            MsgKey::TokenDeployed,
            "トークン'{symbol}'をデプロイしました (ID: {id})".into(),
        );
        m.insert(
            MsgKey::TokenTransferred,
            "{to}に{amount} {symbol}を送信しました".into(),
        );
        m.insert(MsgKey::PasswordPrompt, "パスワードを入力: ".into());
        m.insert(
            MsgKey::BackupCreated,
            "バックアップを{file}に保存しました".into(),
        );
        m.insert(
            MsgKey::BackupRestored,
            "{file}からバックアップを復元しました".into(),
        );
        m.insert(
            MsgKey::SpendingLimitExceeded,
            "支出制限超過: {amount} > {limit}".into(),
        );
        m.insert(
            MsgKey::AddressBlocked,
            "アドレス{address}はブロックされています".into(),
        );
        m.insert(
            MsgKey::MultisigApproved,
            "提案{id}が承認されました ({count}/{threshold})".into(),
        );
        m.insert(MsgKey::NodeConnected, "{url}に接続しました".into());
        m.insert(MsgKey::NodeDisconnected, "ノードから切断されました".into());
        m.insert(MsgKey::SyncComplete, "ブロック{height}で同期完了".into());
        m.insert(
            MsgKey::BridgeInitiated,
            "ブリッジ転送開始: {amount} {token}を{chain}へ".into(),
        );
        m.insert(MsgKey::BridgeCompleted, "ブリッジ転送完了: {id}".into());
        m.insert(
            MsgKey::FaucetReceived,
            "{address}でテストトークンを受け取りました".into(),
        );
        m.insert(
            MsgKey::GasEstimate,
            "推定ガス: {gas} (手数料: {fee} EVAP)".into(),
        );
        m.insert(
            MsgKey::SimulationResult,
            "シミュレーション: 残高{change}、手数料{fee}".into(),
        );
        m.insert(
            MsgKey::HookFired,
            "フック'{name}'が{event}で発火しました".into(),
        );
        m.insert(
            MsgKey::SessionExpired,
            "セッション'{id}'が期限切れです".into(),
        );
        m
    }

    fn chinese() -> HashMap<MsgKey, String> {
        let mut m = HashMap::new();
        m.insert(MsgKey::Welcome, "欢迎使用 EvaporChain 钱包".into());
        m.insert(MsgKey::Error, "错误：{message}".into());
        m.insert(MsgKey::Success, "成功！".into());
        m.insert(MsgKey::Confirm, "确定吗？".into());
        m.insert(MsgKey::Cancel, "已取消。".into());
        m.insert(MsgKey::Yes, "是".into());
        m.insert(MsgKey::No, "否".into());
        m.insert(MsgKey::Loading, "加载中...".into());
        m.insert(MsgKey::Done, "完成。".into());
        m.insert(
            MsgKey::AccountCreated,
            "账户 '{name}' 已创建：{address}".into(),
        );
        m.insert(MsgKey::AccountSwitched, "已切换到账户 '{name}'".into());
        m.insert(MsgKey::AccountBalance, "余额：{amount} EVAP".into());
        m.insert(MsgKey::AccountNotFound, "账户 '{name}' 未找到".into());
        m.insert(
            MsgKey::NoActiveAccount,
            "无活跃账户。运行：wallet account create <名称>".into(),
        );
        m.insert(MsgKey::TransferSent, "已向 {to} 发送 {amount} EVAP".into());
        m.insert(
            MsgKey::TransferConfirmed,
            "转账已在区块 {block} 确认".into(),
        );
        m.insert(MsgKey::TransferFailed, "转账失败：{reason}".into());
        m.insert(
            MsgKey::InsufficientBalance,
            "余额不足：拥有 {have}，需要 {need}".into(),
        );
        m.insert(MsgKey::EnergyLow, "警告：对象 {id} 能量 {pct}%".into());
        m.insert(
            MsgKey::EnergyCritical,
            "严重：对象 {id} 能量 {pct}% — 即将蒸发！".into(),
        );
        m.insert(
            MsgKey::EnergyRefreshed,
            "对象 {id} 已补充 {energy} 能量".into(),
        );
        m.insert(
            MsgKey::EnergyForecast,
            "对象 {id}：能量 {pct}%，约 {epochs} 个纪元后蒸发".into(),
        );
        m.insert(
            MsgKey::ObjectEvaporated,
            "对象 {id} 已蒸发（幽灵状态）".into(),
        );
        m.insert(MsgKey::Staked, "已在池 {pool} 质押 {amount} EVAP".into());
        m.insert(
            MsgKey::Unstaked,
            "已从池 {pool} 解除质押 {amount} EVAP".into(),
        );
        m.insert(
            MsgKey::RewardsClaimed,
            "已从池 {pool} 领取 {amount} EVAP 奖励".into(),
        );
        m.insert(MsgKey::VoteCast, "已对提案 #{id} 投票：{option}".into());
        m.insert(MsgKey::ProposalCreated, "提案 #{id} 已创建：{title}".into());
        m.insert(MsgKey::NftMinted, "NFT '{name}' 已铸造 (ID: {id})".into());
        m.insert(MsgKey::NftTransferred, "NFT {id} 已转移至 {to}".into());
        m.insert(
            MsgKey::TokenDeployed,
            "代币 '{symbol}' 已部署 (ID: {id})".into(),
        );
        m.insert(
            MsgKey::TokenTransferred,
            "已向 {to} 转移 {amount} {symbol}".into(),
        );
        m.insert(MsgKey::PasswordPrompt, "请输入密码：".into());
        m.insert(MsgKey::BackupCreated, "备份已保存至 {file}".into());
        m.insert(MsgKey::BackupRestored, "已从 {file} 恢复备份".into());
        m.insert(
            MsgKey::SpendingLimitExceeded,
            "超出支出限制：{amount} > {limit}".into(),
        );
        m.insert(MsgKey::AddressBlocked, "地址 {address} 已被屏蔽".into());
        m.insert(
            MsgKey::MultisigApproved,
            "提案 {id} 已批准 ({count}/{threshold})".into(),
        );
        m.insert(MsgKey::NodeConnected, "已连接到 {url}".into());
        m.insert(MsgKey::NodeDisconnected, "已断开节点连接".into());
        m.insert(MsgKey::SyncComplete, "区块 {height} 同步完成".into());
        m.insert(
            MsgKey::BridgeInitiated,
            "跨链转账已发起：{amount} {token} 至 {chain}".into(),
        );
        m.insert(MsgKey::BridgeCompleted, "跨链转账完成：{id}".into());
        m.insert(MsgKey::FaucetReceived, "已在 {address} 接收测试代币".into());
        m.insert(
            MsgKey::GasEstimate,
            "估算 Gas：{gas}（费用：{fee} EVAP）".into(),
        );
        m.insert(
            MsgKey::SimulationResult,
            "模拟：余额 {change}，费用 {fee}".into(),
        );
        m.insert(MsgKey::HookFired, "钩子 '{name}' 在 {event} 触发".into());
        m.insert(MsgKey::SessionExpired, "会话 '{id}' 已过期".into());
        m
    }

    fn korean() -> HashMap<MsgKey, String> {
        let mut m = HashMap::new();
        m.insert(
            MsgKey::Welcome,
            "EvaporChain 지갑에 오신 것을 환영합니다".into(),
        );
        m.insert(MsgKey::Error, "오류: {message}".into());
        m.insert(MsgKey::Success, "성공!".into());
        m.insert(MsgKey::Confirm, "확실합니까?".into());
        m.insert(MsgKey::Cancel, "취소되었습니다.".into());
        m.insert(MsgKey::Yes, "예".into());
        m.insert(MsgKey::No, "아니오".into());
        m.insert(MsgKey::Loading, "로딩 중...".into());
        m.insert(MsgKey::Done, "완료.".into());
        m.insert(
            MsgKey::AccountCreated,
            "계정 '{name}'이(가) {address}에 생성되었습니다".into(),
        );
        m.insert(
            MsgKey::AccountSwitched,
            "계정 '{name}'(으)로 전환되었습니다".into(),
        );
        m.insert(MsgKey::AccountBalance, "잔액: {amount} EVAP".into());
        m.insert(
            MsgKey::AccountNotFound,
            "계정 '{name}'을(를) 찾을 수 없습니다".into(),
        );
        m.insert(
            MsgKey::NoActiveAccount,
            "활성 계정이 없습니다. 실행: wallet account create <이름>".into(),
        );
        m.insert(
            MsgKey::TransferSent,
            "{to}에게 {amount} EVAP 전송완료".into(),
        );
        m.insert(
            MsgKey::TransferConfirmed,
            "블록 {block}에서 전송 확인됨".into(),
        );
        m.insert(MsgKey::TransferFailed, "전송 실패: {reason}".into());
        m.insert(
            MsgKey::InsufficientBalance,
            "잔액 부족: {have} 보유, {need} 필요".into(),
        );
        m.insert(MsgKey::EnergyLow, "경고: 객체 {id} 에너지 {pct}%".into());
        m.insert(
            MsgKey::EnergyCritical,
            "위험: 객체 {id} 에너지 {pct}% — 증발 임박!".into(),
        );
        m.insert(
            MsgKey::EnergyRefreshed,
            "객체 {id}에 {energy} 에너지 충전됨".into(),
        );
        m.insert(
            MsgKey::EnergyForecast,
            "객체 {id}: 에너지 {pct}%, 증발까지 ~{epochs} 에포크".into(),
        );
        m.insert(
            MsgKey::ObjectEvaporated,
            "객체 {id}이(가) 증발했습니다 (유령 상태)".into(),
        );
        m.insert(
            MsgKey::Staked,
            "풀 {pool}에 {amount} EVAP 스테이킹됨".into(),
        );
        m.insert(
            MsgKey::Unstaked,
            "풀 {pool}에서 {amount} EVAP 언스테이킹됨".into(),
        );
        m.insert(
            MsgKey::RewardsClaimed,
            "풀 {pool}에서 {amount} EVAP 보상 수령됨".into(),
        );
        m.insert(MsgKey::VoteCast, "제안 #{id}에 투표: {option}".into());
        m.insert(MsgKey::ProposalCreated, "제안 #{id} 생성됨: {title}".into());
        m.insert(MsgKey::NftMinted, "NFT '{name}' 발행됨 (ID: {id})".into());
        m.insert(
            MsgKey::NftTransferred,
            "NFT {id}이(가) {to}에게 전송됨".into(),
        );
        m.insert(
            MsgKey::TokenDeployed,
            "토큰 '{symbol}' 배포됨 (ID: {id})".into(),
        );
        m.insert(
            MsgKey::TokenTransferred,
            "{to}에게 {amount} {symbol} 전송됨".into(),
        );
        m.insert(MsgKey::PasswordPrompt, "비밀번호 입력: ".into());
        m.insert(MsgKey::BackupCreated, "백업이 {file}에 저장됨".into());
        m.insert(MsgKey::BackupRestored, "{file}에서 백업 복원됨".into());
        m.insert(
            MsgKey::SpendingLimitExceeded,
            "지출 한도 초과: {amount} > {limit}".into(),
        );
        m.insert(MsgKey::AddressBlocked, "주소 {address}이(가) 차단됨".into());
        m.insert(
            MsgKey::MultisigApproved,
            "제안 {id} 승인됨 ({count}/{threshold})".into(),
        );
        m.insert(MsgKey::NodeConnected, "{url}에 연결됨".into());
        m.insert(MsgKey::NodeDisconnected, "노드 연결 해제됨".into());
        m.insert(MsgKey::SyncComplete, "블록 {height}에서 동기화 완료".into());
        m.insert(
            MsgKey::BridgeInitiated,
            "브릿지 전송 시작: {amount} {token} → {chain}".into(),
        );
        m.insert(MsgKey::BridgeCompleted, "브릿지 전송 완료: {id}".into());
        m.insert(
            MsgKey::FaucetReceived,
            "{address}에서 테스트 토큰 수령됨".into(),
        );
        m.insert(
            MsgKey::GasEstimate,
            "예상 가스: {gas} (수수료: {fee} EVAP)".into(),
        );
        m.insert(
            MsgKey::SimulationResult,
            "시뮬레이션: 잔액 {change}, 수수료 {fee}".into(),
        );
        m.insert(
            MsgKey::HookFired,
            "훅 '{name}'이(가) {event}에서 실행됨".into(),
        );
        m.insert(MsgKey::SessionExpired, "세션 '{id}' 만료됨".into());
        m
    }

    fn portuguese() -> HashMap<MsgKey, String> {
        let mut m = HashMap::new();
        m.insert(MsgKey::Welcome, "Bem-vindo ao EvaporChain Wallet".into());
        m.insert(MsgKey::Error, "Erro: {message}".into());
        m.insert(MsgKey::Success, "Sucesso!".into());
        m.insert(MsgKey::Confirm, "Tem certeza?".into());
        m.insert(MsgKey::Cancel, "Cancelado.".into());
        m.insert(MsgKey::Yes, "Sim".into());
        m.insert(MsgKey::No, "Não".into());
        m.insert(MsgKey::Loading, "Carregando...".into());
        m.insert(MsgKey::Done, "Concluído.".into());
        m.insert(
            MsgKey::AccountCreated,
            "Conta '{name}' criada em {address}".into(),
        );
        m.insert(
            MsgKey::AccountSwitched,
            "Alternado para conta '{name}'".into(),
        );
        m.insert(MsgKey::AccountBalance, "Saldo: {amount} EVAP".into());
        m.insert(
            MsgKey::AccountNotFound,
            "Conta '{name}' não encontrada".into(),
        );
        m.insert(
            MsgKey::NoActiveAccount,
            "Nenhuma conta ativa. Execute: wallet account create <nome>".into(),
        );
        m.insert(
            MsgKey::TransferSent,
            "{amount} EVAP enviados para {to}".into(),
        );
        m.insert(
            MsgKey::TransferConfirmed,
            "Transferência confirmada no bloco {block}".into(),
        );
        m.insert(
            MsgKey::TransferFailed,
            "Transferência falhou: {reason}".into(),
        );
        m.insert(
            MsgKey::InsufficientBalance,
            "Saldo insuficiente: tem {have}, precisa {need}".into(),
        );
        m.insert(
            MsgKey::EnergyLow,
            "Aviso: Objeto {id} energia em {pct}%".into(),
        );
        m.insert(
            MsgKey::EnergyCritical,
            "CRÍTICO: Objeto {id} energia em {pct}% — evaporação iminente!".into(),
        );
        m.insert(
            MsgKey::EnergyRefreshed,
            "Objeto {id} recarregado com {energy} energia".into(),
        );
        m.insert(
            MsgKey::EnergyForecast,
            "Objeto {id}: {pct}% energia, ~{epochs} épocas até evaporação".into(),
        );
        m.insert(
            MsgKey::ObjectEvaporated,
            "Objeto {id} evaporou (estado fantasma)".into(),
        );
        m.insert(MsgKey::Staked, "{amount} EVAP staked na pool {pool}".into());
        m.insert(
            MsgKey::Unstaked,
            "{amount} EVAP unstaked da pool {pool}".into(),
        );
        m.insert(
            MsgKey::RewardsClaimed,
            "{amount} EVAP de recompensas coletadas da pool {pool}".into(),
        );
        m.insert(
            MsgKey::VoteCast,
            "Voto registrado na proposta #{id}: {option}".into(),
        );
        m.insert(
            MsgKey::ProposalCreated,
            "Proposta #{id} criada: {title}".into(),
        );
        m.insert(MsgKey::NftMinted, "NFT '{name}' cunhado (ID: {id})".into());
        m.insert(
            MsgKey::NftTransferred,
            "NFT {id} transferido para {to}".into(),
        );
        m.insert(
            MsgKey::TokenDeployed,
            "Token '{symbol}' implantado (ID: {id})".into(),
        );
        m.insert(
            MsgKey::TokenTransferred,
            "{amount} {symbol} transferidos para {to}".into(),
        );
        m.insert(MsgKey::PasswordPrompt, "Digite a senha: ".into());
        m.insert(MsgKey::BackupCreated, "Backup salvo em {file}".into());
        m.insert(MsgKey::BackupRestored, "Backup restaurado de {file}".into());
        m.insert(
            MsgKey::SpendingLimitExceeded,
            "Limite de gasto excedido: {amount} > {limit}".into(),
        );
        m.insert(
            MsgKey::AddressBlocked,
            "Endereço {address} bloqueado".into(),
        );
        m.insert(
            MsgKey::MultisigApproved,
            "Proposta {id} aprovada ({count}/{threshold})".into(),
        );
        m.insert(MsgKey::NodeConnected, "Conectado a {url}".into());
        m.insert(MsgKey::NodeDisconnected, "Desconectado do nó".into());
        m.insert(
            MsgKey::SyncComplete,
            "Sincronização completa no bloco {height}".into(),
        );
        m.insert(
            MsgKey::BridgeInitiated,
            "Transferência bridge iniciada: {amount} {token} para {chain}".into(),
        );
        m.insert(
            MsgKey::BridgeCompleted,
            "Transferência bridge concluída: {id}".into(),
        );
        m.insert(
            MsgKey::FaucetReceived,
            "Tokens de teste recebidos em {address}".into(),
        );
        m.insert(
            MsgKey::GasEstimate,
            "Gas estimado: {gas} (taxa: {fee} EVAP)".into(),
        );
        m.insert(
            MsgKey::SimulationResult,
            "Simulação: saldo {change}, taxa {fee}".into(),
        );
        m.insert(
            MsgKey::HookFired,
            "Hook '{name}' disparado em {event}".into(),
        );
        m.insert(MsgKey::SessionExpired, "Sessão '{id}' expirada".into());
        m
    }

    fn russian() -> HashMap<MsgKey, String> {
        let mut m = HashMap::new();
        m.insert(
            MsgKey::Welcome,
            "Добро пожаловать в EvaporChain Wallet".into(),
        );
        m.insert(MsgKey::Error, "Ошибка: {message}".into());
        m.insert(MsgKey::Success, "Успех!".into());
        m.insert(MsgKey::Confirm, "Вы уверены?".into());
        m.insert(MsgKey::Cancel, "Отменено.".into());
        m.insert(MsgKey::Yes, "Да".into());
        m.insert(MsgKey::No, "Нет".into());
        m.insert(MsgKey::Loading, "Загрузка...".into());
        m.insert(MsgKey::Done, "Готово.".into());
        m.insert(
            MsgKey::AccountCreated,
            "Аккаунт '{name}' создан: {address}".into(),
        );
        m.insert(
            MsgKey::AccountSwitched,
            "Переключено на аккаунт '{name}'".into(),
        );
        m.insert(MsgKey::AccountBalance, "Баланс: {amount} EVAP".into());
        m.insert(MsgKey::AccountNotFound, "Аккаунт '{name}' не найден".into());
        m.insert(
            MsgKey::NoActiveAccount,
            "Нет активного аккаунта. Выполните: wallet account create <имя>".into(),
        );
        m.insert(
            MsgKey::TransferSent,
            "Отправлено {amount} EVAP на {to}".into(),
        );
        m.insert(
            MsgKey::TransferConfirmed,
            "Перевод подтверждён в блоке {block}".into(),
        );
        m.insert(MsgKey::TransferFailed, "Перевод не удался: {reason}".into());
        m.insert(
            MsgKey::InsufficientBalance,
            "Недостаточный баланс: есть {have}, нужно {need}".into(),
        );
        m.insert(
            MsgKey::EnergyLow,
            "Внимание: Объект {id} энергия {pct}%".into(),
        );
        m.insert(
            MsgKey::EnergyCritical,
            "КРИТИЧНО: Объект {id} энергия {pct}% — испарение неизбежно!".into(),
        );
        m.insert(
            MsgKey::EnergyRefreshed,
            "Объект {id} пополнен на {energy} энергии".into(),
        );
        m.insert(
            MsgKey::EnergyForecast,
            "Объект {id}: энергия {pct}%, ~{epochs} эпох до испарения".into(),
        );
        m.insert(
            MsgKey::ObjectEvaporated,
            "Объект {id} испарился (состояние призрака)".into(),
        );
        m.insert(
            MsgKey::Staked,
            "{amount} EVAP застейкано в пуле {pool}".into(),
        );
        m.insert(
            MsgKey::Unstaked,
            "{amount} EVAP выведено из пула {pool}".into(),
        );
        m.insert(
            MsgKey::RewardsClaimed,
            "Получено {amount} EVAP наград из пула {pool}".into(),
        );
        m.insert(
            MsgKey::VoteCast,
            "Голос за предложение #{id}: {option}".into(),
        );
        m.insert(
            MsgKey::ProposalCreated,
            "Предложение #{id} создано: {title}".into(),
        );
        m.insert(MsgKey::NftMinted, "NFT '{name}' создан (ID: {id})".into());
        m.insert(MsgKey::NftTransferred, "NFT {id} передан {to}".into());
        m.insert(
            MsgKey::TokenDeployed,
            "Токен '{symbol}' развёрнут (ID: {id})".into(),
        );
        m.insert(
            MsgKey::TokenTransferred,
            "Переведено {amount} {symbol} на {to}".into(),
        );
        m.insert(MsgKey::PasswordPrompt, "Введите пароль: ".into());
        m.insert(
            MsgKey::BackupCreated,
            "Резервная копия сохранена в {file}".into(),
        );
        m.insert(
            MsgKey::BackupRestored,
            "Резервная копия восстановлена из {file}".into(),
        );
        m.insert(
            MsgKey::SpendingLimitExceeded,
            "Лимит расходов превышен: {amount} > {limit}".into(),
        );
        m.insert(
            MsgKey::AddressBlocked,
            "Адрес {address} заблокирован".into(),
        );
        m.insert(
            MsgKey::MultisigApproved,
            "Предложение {id} одобрено ({count}/{threshold})".into(),
        );
        m.insert(MsgKey::NodeConnected, "Подключено к {url}".into());
        m.insert(MsgKey::NodeDisconnected, "Отключено от узла".into());
        m.insert(
            MsgKey::SyncComplete,
            "Синхронизация завершена на блоке {height}".into(),
        );
        m.insert(
            MsgKey::BridgeInitiated,
            "Мост-перевод начат: {amount} {token} в {chain}".into(),
        );
        m.insert(
            MsgKey::BridgeCompleted,
            "Мост-перевод завершён: {id}".into(),
        );
        m.insert(
            MsgKey::FaucetReceived,
            "Тестовые токены получены на {address}".into(),
        );
        m.insert(
            MsgKey::GasEstimate,
            "Оценка газа: {gas} (комиссия: {fee} EVAP)".into(),
        );
        m.insert(
            MsgKey::SimulationResult,
            "Симуляция: баланс {change}, комиссия {fee}".into(),
        );
        m.insert(MsgKey::HookFired, "Хук '{name}' сработал на {event}".into());
        m.insert(MsgKey::SessionExpired, "Сессия '{id}' истекла".into());
        m
    }

    fn arabic() -> HashMap<MsgKey, String> {
        let mut m = HashMap::new();
        m.insert(MsgKey::Welcome, "مرحبًا بكم في محفظة EvaporChain".into());
        m.insert(MsgKey::Error, "خطأ: {message}".into());
        m.insert(MsgKey::Success, "نجاح!".into());
        m.insert(MsgKey::Confirm, "هل أنت متأكد؟".into());
        m.insert(MsgKey::Cancel, "تم الإلغاء.".into());
        m.insert(MsgKey::Yes, "نعم".into());
        m.insert(MsgKey::No, "لا".into());
        m.insert(MsgKey::Loading, "جارٍ التحميل...".into());
        m.insert(MsgKey::Done, "تم.".into());
        m.insert(
            MsgKey::AccountCreated,
            "تم إنشاء الحساب '{name}' في {address}".into(),
        );
        m.insert(
            MsgKey::AccountSwitched,
            "تم التبديل إلى الحساب '{name}'".into(),
        );
        m.insert(MsgKey::AccountBalance, "الرصيد: {amount} EVAP".into());
        m.insert(MsgKey::AccountNotFound, "الحساب '{name}' غير موجود".into());
        m.insert(
            MsgKey::NoActiveAccount,
            "لا يوجد حساب نشط. نفّذ: wallet account create <اسم>".into(),
        );
        m.insert(
            MsgKey::TransferSent,
            "تم إرسال {amount} EVAP إلى {to}".into(),
        );
        m.insert(
            MsgKey::TransferConfirmed,
            "تم تأكيد التحويل في الكتلة {block}".into(),
        );
        m.insert(MsgKey::TransferFailed, "فشل التحويل: {reason}".into());
        m.insert(
            MsgKey::InsufficientBalance,
            "رصيد غير كافٍ: لديك {have}، تحتاج {need}".into(),
        );
        m.insert(
            MsgKey::EnergyLow,
            "تحذير: الكائن {id} الطاقة عند {pct}%".into(),
        );
        m.insert(
            MsgKey::EnergyCritical,
            "حرج: الكائن {id} الطاقة عند {pct}% — التبخر وشيك!".into(),
        );
        m.insert(
            MsgKey::EnergyRefreshed,
            "تم شحن الكائن {id} بـ {energy} طاقة".into(),
        );
        m.insert(
            MsgKey::EnergyForecast,
            "الكائن {id}: طاقة {pct}%، ~{epochs} حقبة حتى التبخر".into(),
        );
        m.insert(
            MsgKey::ObjectEvaporated,
            "الكائن {id} تبخر (حالة شبح)".into(),
        );
        m.insert(
            MsgKey::Staked,
            "تم تخزين {amount} EVAP في المجمع {pool}".into(),
        );
        m.insert(
            MsgKey::Unstaked,
            "تم سحب {amount} EVAP من المجمع {pool}".into(),
        );
        m.insert(
            MsgKey::RewardsClaimed,
            "تم استلام {amount} EVAP مكافآت من المجمع {pool}".into(),
        );
        m.insert(
            MsgKey::VoteCast,
            "تم التصويت على الاقتراح #{id}: {option}".into(),
        );
        m.insert(
            MsgKey::ProposalCreated,
            "تم إنشاء الاقتراح #{id}: {title}".into(),
        );
        m.insert(
            MsgKey::NftMinted,
            "تم سك NFT '{name}' (المعرف: {id})".into(),
        );
        m.insert(MsgKey::NftTransferred, "تم نقل NFT {id} إلى {to}".into());
        m.insert(
            MsgKey::TokenDeployed,
            "تم نشر الرمز '{symbol}' (المعرف: {id})".into(),
        );
        m.insert(
            MsgKey::TokenTransferred,
            "تم تحويل {amount} {symbol} إلى {to}".into(),
        );
        m.insert(MsgKey::PasswordPrompt, "أدخل كلمة المرور: ".into());
        m.insert(
            MsgKey::BackupCreated,
            "تم حفظ النسخة الاحتياطية في {file}".into(),
        );
        m.insert(
            MsgKey::BackupRestored,
            "تم استعادة النسخة الاحتياطية من {file}".into(),
        );
        m.insert(
            MsgKey::SpendingLimitExceeded,
            "تم تجاوز حد الإنفاق: {amount} > {limit}".into(),
        );
        m.insert(MsgKey::AddressBlocked, "العنوان {address} محظور".into());
        m.insert(
            MsgKey::MultisigApproved,
            "تمت الموافقة على الاقتراح {id} ({count}/{threshold})".into(),
        );
        m.insert(MsgKey::NodeConnected, "متصل بـ {url}".into());
        m.insert(MsgKey::NodeDisconnected, "تم قطع الاتصال بالعقدة".into());
        m.insert(
            MsgKey::SyncComplete,
            "اكتملت المزامنة عند الكتلة {height}".into(),
        );
        m.insert(
            MsgKey::BridgeInitiated,
            "بدأ تحويل الجسر: {amount} {token} إلى {chain}".into(),
        );
        m.insert(MsgKey::BridgeCompleted, "اكتمل تحويل الجسر: {id}".into());
        m.insert(
            MsgKey::FaucetReceived,
            "تم استلام رموز الاختبار في {address}".into(),
        );
        m.insert(
            MsgKey::GasEstimate,
            "تقدير الغاز: {gas} (الرسوم: {fee} EVAP)".into(),
        );
        m.insert(
            MsgKey::SimulationResult,
            "المحاكاة: الرصيد {change}، الرسوم {fee}".into(),
        );
        m.insert(
            MsgKey::HookFired,
            "تم تنشيط الخطاف '{name}' في {event}".into(),
        );
        m.insert(MsgKey::SessionExpired, "انتهت صلاحية الجلسة '{id}'".into());
        m
    }
}

impl Default for I18n {
    fn default() -> Self {
        Self::new()
    }
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_defaults_to_english() {
        let i18n = I18n::new();
        assert_eq!(i18n.locale(), Locale::En);
    }

    #[test]
    fn test_with_locale() {
        let i18n = I18n::with_locale(Locale::Es);
        assert_eq!(i18n.locale(), Locale::Es);
    }

    #[test]
    fn test_set_locale() {
        let mut i18n = I18n::new();
        i18n.set_locale(Locale::Fr);
        assert_eq!(i18n.locale(), Locale::Fr);
    }

    #[test]
    fn test_set_locale_str_valid() {
        let mut i18n = I18n::new();
        assert!(i18n.set_locale_str("es").is_ok());
        assert_eq!(i18n.locale(), Locale::Es);
    }

    #[test]
    fn test_set_locale_str_invalid() {
        let mut i18n = I18n::new();
        assert!(i18n.set_locale_str("xx").is_err());
    }

    #[test]
    fn test_locale_from_str_with_region() {
        assert_eq!(Locale::from_str("en-US"), Some(Locale::En));
        assert_eq!(Locale::from_str("zh-CN"), Some(Locale::Zh));
        assert_eq!(Locale::from_str("pt_BR"), Some(Locale::Pt));
        assert_eq!(Locale::from_str("fr-FR"), Some(Locale::Fr));
    }

    #[test]
    fn test_locale_from_str_case_insensitive() {
        assert_eq!(Locale::from_str("EN"), Some(Locale::En));
        assert_eq!(Locale::from_str("JA"), Some(Locale::Ja));
    }

    #[test]
    fn test_get_english() {
        let i18n = I18n::new();
        assert_eq!(i18n.get(MsgKey::Welcome), "Welcome to EvaporChain Wallet");
        assert_eq!(i18n.get(MsgKey::Success), "Success!");
    }

    #[test]
    fn test_get_spanish() {
        let i18n = I18n::with_locale(Locale::Es);
        assert_eq!(i18n.get(MsgKey::Welcome), "Bienvenido a EvaporChain Wallet");
        assert_eq!(i18n.get(MsgKey::Success), "¡Éxito!");
    }

    #[test]
    fn test_get_french() {
        let i18n = I18n::with_locale(Locale::Fr);
        assert_eq!(
            i18n.get(MsgKey::Welcome),
            "Bienvenue sur EvaporChain Wallet"
        );
    }

    #[test]
    fn test_get_german() {
        let i18n = I18n::with_locale(Locale::De);
        assert_eq!(
            i18n.get(MsgKey::Welcome),
            "Willkommen bei EvaporChain Wallet"
        );
    }

    #[test]
    fn test_get_japanese() {
        let i18n = I18n::with_locale(Locale::Ja);
        assert_eq!(i18n.get(MsgKey::Welcome), "EvaporChain Walletへようこそ");
    }

    #[test]
    fn test_get_chinese() {
        let i18n = I18n::with_locale(Locale::Zh);
        assert_eq!(i18n.get(MsgKey::Welcome), "欢迎使用 EvaporChain 钱包");
    }

    #[test]
    fn test_get_korean() {
        let i18n = I18n::with_locale(Locale::Ko);
        assert_eq!(
            i18n.get(MsgKey::Welcome),
            "EvaporChain 지갑에 오신 것을 환영합니다"
        );
    }

    #[test]
    fn test_get_hindi() {
        let i18n = I18n::with_locale(Locale::Hi);
        assert_eq!(
            i18n.get(MsgKey::Welcome),
            "EvaporChain Wallet में आपका स्वागत है"
        );
    }

    #[test]
    fn test_get_portuguese() {
        let i18n = I18n::with_locale(Locale::Pt);
        assert_eq!(i18n.get(MsgKey::Welcome), "Bem-vindo ao EvaporChain Wallet");
    }

    #[test]
    fn test_get_russian() {
        let i18n = I18n::with_locale(Locale::Ru);
        assert_eq!(
            i18n.get(MsgKey::Welcome),
            "Добро пожаловать в EvaporChain Wallet"
        );
    }

    #[test]
    fn test_get_arabic() {
        let i18n = I18n::with_locale(Locale::Ar);
        assert_eq!(i18n.get(MsgKey::Welcome), "مرحبًا بكم في محفظة EvaporChain");
    }

    #[test]
    fn test_format_interpolation() {
        let i18n = I18n::new();
        let msg = i18n.format(MsgKey::TransferSent, &[("amount", "1000"), ("to", "0xabc")]);
        assert_eq!(msg, "Sent 1000 EVAP to 0xabc");
    }

    #[test]
    fn test_format_interpolation_spanish() {
        let i18n = I18n::with_locale(Locale::Es);
        let msg = i18n.format(MsgKey::TransferSent, &[("amount", "500"), ("to", "0xdef")]);
        assert_eq!(msg, "Enviados 500 EVAP a 0xdef");
    }

    #[test]
    fn test_format_multiple_vars() {
        let i18n = I18n::new();
        let msg = i18n.format(
            MsgKey::InsufficientBalance,
            &[("have", "100"), ("need", "500")],
        );
        assert_eq!(msg, "Insufficient balance: have 100, need 500");
    }

    #[test]
    fn test_format_energy_critical() {
        let i18n = I18n::new();
        let msg = i18n.format(MsgKey::EnergyCritical, &[("id", "obj_42"), ("pct", "3")]);
        assert_eq!(
            msg,
            "CRITICAL: Object obj_42 energy at 3% — evaporation imminent!"
        );
    }

    #[test]
    fn test_supported_locales() {
        let i18n = I18n::new();
        let locales = i18n.supported_locales();
        assert_eq!(locales.len(), 11);
        assert_eq!(locales[0], (Locale::En, "English"));
        assert_eq!(locales[1], (Locale::Es, "Español"));
    }

    #[test]
    fn test_all_locales_complete() {
        let i18n = I18n::new();
        for locale in Locale::all() {
            assert!(
                i18n.is_complete(*locale),
                "Locale {} is not complete",
                locale.native_name()
            );
        }
    }

    #[test]
    fn test_completeness_percentage() {
        let i18n = I18n::new();
        assert_eq!(i18n.completeness(Locale::En), 100.0);
        assert_eq!(i18n.completeness(Locale::Es), 100.0);
    }

    #[test]
    fn test_set_custom_translation() {
        let mut i18n = I18n::new();
        i18n.set_translation(Locale::En, MsgKey::Welcome, "Hello World!".into());
        assert_eq!(i18n.get(MsgKey::Welcome), "Hello World!");
    }

    #[test]
    fn test_locale_code() {
        assert_eq!(Locale::En.code(), "en");
        assert_eq!(Locale::Ja.code(), "ja");
        assert_eq!(Locale::Zh.code(), "zh");
    }

    #[test]
    fn test_locale_native_name() {
        assert_eq!(Locale::En.native_name(), "English");
        assert_eq!(Locale::Hi.native_name(), "हिन्दी");
        assert_eq!(Locale::Ar.native_name(), "العربية");
    }

    #[test]
    fn test_locale_display() {
        assert_eq!(format!("{}", Locale::En), "en");
        assert_eq!(format!("{}", Locale::Es), "es");
    }

    #[test]
    fn test_locale_from_str_unknown() {
        assert_eq!(Locale::from_str("xx"), None);
        assert_eq!(Locale::from_str(""), None);
    }

    #[test]
    fn test_format_with_missing_placeholder() {
        let i18n = I18n::new();
        let msg = i18n.format(MsgKey::TransferSent, &[("amount", "100")]);
        // {to} remains as literal since no value provided
        assert!(msg.contains("100"));
        assert!(msg.contains("{to}"));
    }

    #[test]
    fn test_default_trait() {
        let i18n = I18n::default();
        assert_eq!(i18n.locale(), Locale::En);
    }

    #[test]
    fn test_all_message_keys_present_in_english() {
        let i18n = I18n::new();
        let keys = [
            MsgKey::Welcome,
            MsgKey::Error,
            MsgKey::Success,
            MsgKey::Confirm,
            MsgKey::Cancel,
            MsgKey::Yes,
            MsgKey::No,
            MsgKey::Loading,
            MsgKey::Done,
            MsgKey::AccountCreated,
            MsgKey::AccountSwitched,
            MsgKey::AccountBalance,
            MsgKey::AccountNotFound,
            MsgKey::NoActiveAccount,
            MsgKey::TransferSent,
            MsgKey::TransferConfirmed,
            MsgKey::TransferFailed,
            MsgKey::InsufficientBalance,
            MsgKey::EnergyLow,
            MsgKey::EnergyCritical,
            MsgKey::EnergyRefreshed,
            MsgKey::EnergyForecast,
            MsgKey::ObjectEvaporated,
            MsgKey::Staked,
            MsgKey::Unstaked,
            MsgKey::RewardsClaimed,
            MsgKey::VoteCast,
            MsgKey::ProposalCreated,
            MsgKey::NftMinted,
            MsgKey::NftTransferred,
            MsgKey::TokenDeployed,
            MsgKey::TokenTransferred,
            MsgKey::PasswordPrompt,
            MsgKey::BackupCreated,
            MsgKey::BackupRestored,
            MsgKey::SpendingLimitExceeded,
            MsgKey::AddressBlocked,
            MsgKey::MultisigApproved,
            MsgKey::NodeConnected,
            MsgKey::NodeDisconnected,
            MsgKey::SyncComplete,
            MsgKey::BridgeInitiated,
            MsgKey::BridgeCompleted,
            MsgKey::FaucetReceived,
            MsgKey::GasEstimate,
            MsgKey::SimulationResult,
            MsgKey::HookFired,
            MsgKey::SessionExpired,
        ];
        for key in &keys {
            let msg = i18n.get(*key);
            assert_ne!(msg, "???", "Missing key {:?} in English", key);
        }
    }
}
