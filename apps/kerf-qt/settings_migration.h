#pragma once

// The editor's settings carry a version, settings/version, and each step from
// one version to the next is a migration here, run once at start before
// anything reads them. Settings from a newer Kerf are left as they are.

class QSettings;

namespace kerf {

// The version this editor writes.
constexpr int kSettingsVersion = 1;

// Brings `settings` up to kSettingsVersion. Returns the version it found:
// 0 for settings from before versions, or one newer than this editor's,
// which it leaves untouched.
int migrateSettings(QSettings &settings);

} // namespace kerf
