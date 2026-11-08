#include "settings_migration.h"

#include <QSettings>
#include <QStringList>

namespace kerf {

namespace {
const QString kVersionKey = QStringLiteral("settings/version");

// 0 -> 1: what no code reads any more, and a model nobody chose.
void toVersion1(QSettings &settings) {
    // The workspaces, the viewer's chat and device previews, and the
    // Inspector's follow-selection, all since replaced (stage_shell_test's
    // oldSettingsAreIgnored).
    for (const QString &key : {QStringLiteral("workspace/current"), QStringLiteral("workspace/names"),
                               QStringLiteral("workspace/changes"), QStringLiteral("workspace/stage"),
                               QStringLiteral("viewer/chatOpen"), QStringLiteral("viewer/chatWidth"),
                               QStringLiteral("viewer/devicePreview"), QStringLiteral("viewer/laptopPreview"),
                               QStringLiteral("editor/inspectorFollowsSelection")})
        settings.remove(key);
    // Before c58bca5 the editor wrote its own default model into the
    // settings on every save, so a person who never chose one is held to
    // that retired model. Clear it, and kerf-agent's default applies; a
    // model someone typed is kept.
    const QString model = settings.value(QStringLiteral("agent/anthropic/model")).toString().trimmed();
    if (model == QStringLiteral("claude-sonnet-4-20250514") || model == QStringLiteral("claude-3-5-sonnet-latest"))
        settings.remove(QStringLiteral("agent/anthropic/model"));
}
} // namespace

int migrateSettings(QSettings &settings) {
    const int found = settings.value(kVersionKey, 0).toInt();
    if (found >= kSettingsVersion) return found;
    if (found < 1) toVersion1(settings);
    settings.setValue(kVersionKey, kSettingsVersion);
    settings.sync();
    return found;
}

} // namespace kerf
