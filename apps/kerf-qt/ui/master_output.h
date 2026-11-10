#pragma once

// Where Export Master renders: the project's master output. It goes to
// deliveries/ in the project as <project>_master, ProRes 422 HQ, unless the
// project's settings name another folder, file name or codec for it (kept by
// the project's absolute path under deliver/<folder>~<digest>/outputs, as
// {master: {folder?, name?, codec?}}).

#include <QJsonObject>
#include <QString>

namespace kerf::deliver {

// The project's folder name ("atlas"); empty without a project.
QString projectName(const QString &directory);
// The QSettings key of `key` for the project, by its absolute path:
// deliver/<folder name>~<12 hex of the path's SHA-1>/<key>. Empty without a
// project, which keeps nothing.
QString settingsKey(const QString &directory, const QString &key);
// The export command that renders the project's master: {type: export,
// quality: master, codec? (left out when it is ProRes 422 HQ), output?, name?}.
QJsonObject masterCommand(const QString &directory);

} // namespace kerf::deliver
