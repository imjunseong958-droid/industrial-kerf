#pragma once

// Whether a newer Kerf is out (kerf_update_check asks GitHub's releases), and
// telling the person once: Download opens the release page, Skip This Version
// stops asking about that one. Kerf never downloads or installs anything
// itself. Asked at most once a day; updates/check=false turns it off.

#include <QDateTime>
#include <QJsonObject>
#include <QString>

#include <optional>

class QSettings;
class QWidget;

namespace kerf {

struct UpdateOffer {
    QString version;
    QString url;
};

// Whether to ask GitHub now: on, and not asked in the last day.
bool shouldCheckForUpdates(const QSettings &settings, const QDateTime &now);
// What to offer from kerf_update_check's answer: a newer release that was
// not skipped.
std::optional<UpdateOffer> updateOffer(const QJsonObject &answer, const QSettings &settings);

// Asks off the GUI thread when it is time, and offers what it finds over
// `parent`.
void startUpdateCheck(QWidget *parent);

} // namespace kerf
