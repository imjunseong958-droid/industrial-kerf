#include "user_guide.h"

#include <QDir>
#include <QFileInfo>
#include <QStringList>

namespace kerf {

QUrl onlineUserGuideUrl() {
    return QUrl(QStringLiteral("https://github.com/imjunseong958-droid/industrial-kerf/blob/main/docs/user-guide.md"));
}

QString localUserGuide(const QString &applicationDirectory) {
    if (applicationDirectory.isEmpty()) return {};
    const QDir here(applicationDirectory);
    // Where scripts/package-kerf-windows.ps1, package-kerf-macos.sh and the
    // Linux package put it, in that order.
    const QStringList places{
        QStringLiteral("user-guide.html"),
        QStringLiteral("../Resources/user-guide.html"),
        QStringLiteral("../share/doc/industrial-kerf/user-guide.html"),
    };
    for (const QString &place : places) {
        const QFileInfo page(here.filePath(place));
        if (page.isFile() && page.size() > 0) return QDir::cleanPath(page.absoluteFilePath());
    }
    return {};
}

QUrl userGuideUrl(const QString &applicationDirectory) {
    const QString local = localUserGuide(applicationDirectory);
    return local.isEmpty() ? onlineUserGuideUrl() : QUrl::fromLocalFile(local);
}

} // namespace kerf
