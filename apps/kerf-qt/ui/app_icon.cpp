#include "app_icon.h"

#include <QSize>
#include <QString>

namespace kerf {

QList<int> appIconSides() { return {16, 24, 32, 48, 64, 128, 256}; }

QIcon appIcon() {
    QIcon icon;
    for (const int side : appIconSides())
        icon.addFile(QStringLiteral(":/icon/industrial-kerf-%1.png").arg(side), QSize(side, side));
    return icon;
}

} // namespace kerf
