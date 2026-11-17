#pragma once

// A making Kerf asked the person to run (the bridge's agent_ask, and an
// answer's asks in a kept conversation): the media, how many takes (1 unless
// said), the first of them, and the plan as `make` said it. Plain data: the
// conversation model reads it, MakeCard shows it.

#include <QJsonObject>
#include <QString>

#include <algorithm>

namespace kerf {

struct MakingAsk {
    QString media;
    int rolls = 1;
    int take = 0;
    QString text;
    // A making asked for twice is one card.
    QString key() const { return QStringLiteral("%1#%2").arg(media).arg(take); }
    bool isValid() const { return !media.isEmpty(); }

    static MakingAsk read(const QJsonObject &object) {
        MakingAsk ask;
        ask.media = object.value(QStringLiteral("media")).toString();
        ask.rolls = std::max(1, object.value(QStringLiteral("rolls")).toInt(1));
        ask.take = object.value(QStringLiteral("take")).toInt();
        ask.text = object.value(QStringLiteral("text")).toString();
        return ask;
    }
};

} // namespace kerf
