#pragma once

// API keys in the system keychain (kerf_secret: macOS Keychain, Windows
// Credential Manager, the Secret Service on Linux) rather than in plain
// QSettings. Where the keychain cannot be reached, keys stay in the settings
// as before; a key found there is moved into the keychain once it can be.

#include <QJsonObject>
#include <QString>

#include <functional>

class QSettings;

namespace kerf::secrets {

// How a request reaches the keychain: {op, name, value?} -> {ok, value?} or
// {error}. The bridge's kerf_secret; tests replace it.
using Call = std::function<QJsonObject(const QJsonObject &)>;
Call &call();

// The key kept as `key` (a settings key such as agent/anthropic/apiKey): from
// the keychain, else from the settings, which it then moves into the
// keychain, else `fallback`.
QString loadKey(QSettings &settings, const QString &key, const QString &fallback = QString());
// Keeps `value` in the keychain and out of the settings, or in the settings
// when the keychain cannot be reached. An empty value forgets the key.
void saveKey(QSettings &settings, const QString &key, const QString &value);

} // namespace kerf::secrets
