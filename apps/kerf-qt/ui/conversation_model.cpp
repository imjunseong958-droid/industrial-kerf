#include "conversation_model.h"

#include <QJsonArray>

#include <algorithm>

namespace kerf {

namespace {

ConversationModel::Summary readSummary(const QJsonObject &object) {
    ConversationModel::Summary summary;
    summary.id = object.value(QStringLiteral("id")).toString();
    summary.title = object.value(QStringLiteral("title")).toString();
    summary.created = ConversationModel::readTime(object.value(QStringLiteral("created")).toString());
    summary.updated = ConversationModel::readTime(object.value(QStringLiteral("updated")).toString());
    summary.archived = object.value(QStringLiteral("archived")).toBool();
    summary.count = object.value(QStringLiteral("count")).toInt(int(object.value(QStringLiteral("items")).toArray().size()));
    return summary;
}

} // namespace

ConversationModel::ConversationModel(QObject *parent) : QObject(parent) {}

QDateTime ConversationModel::readTime(const QString &text) {
    QDateTime time = QDateTime::fromString(text, Qt::ISODateWithMs);
    if (!time.isValid()) time = QDateTime::fromString(text, Qt::ISODate);
    return time;
}

ConversationModel::Item ConversationModel::readItem(const QJsonObject &object) {
    Item item;
    item.id = object.value(QStringLiteral("id")).toString();
    item.you = object.value(QStringLiteral("kind")).toString() == QLatin1String("you");
    item.text = object.value(QStringLiteral("text")).toString();
    item.at = readTime(object.value(QStringLiteral("at")).toString());
    const QJsonValue timecode = object.value(QStringLiteral("timecode"));
    if (timecode.isDouble()) item.timecode = timecode.toDouble();
    for (const QJsonValue &step : object.value(QStringLiteral("steps")).toArray())
        if (!step.toString().isEmpty()) item.steps << step.toString();
    item.seconds = object.value(QStringLiteral("seconds")).toDouble();
    item.error = object.value(QStringLiteral("error")).toBool();
    item.stopped = object.value(QStringLiteral("stopped")).toBool();
    item.proposal = object.value(QStringLiteral("proposal")).toString();
    for (const QJsonValue &value : object.value(QStringLiteral("asks")).toArray())
        if (const MakingAsk ask = MakingAsk::read(value.toObject()); ask.isValid()) item.asks << ask;
    return item;
}

const ConversationModel::Summary *ConversationModel::summary(const QString &id) const {
    for (const Summary &summary : m_list)
        if (summary.id == id) return &summary;
    return nullptr;
}

void ConversationModel::choose(const QString &id) {
    m_expectNew = false;
    m_chosen = id;
    if (id == m_current.summary.id) return;
    // Until the bridge sends it, the panel shows the conversation's title alone.
    m_current = Conversation{};
    if (const Summary *known = summary(id)) m_current.summary = *known;
    else m_current.summary.id = id;
    if (id.isEmpty()) m_current = Conversation{};
    emit currentChanged();
}

bool ConversationModel::applyEvent(const QString &type, const QJsonObject &event) {
    if (type == QLatin1String("conversations")) {
        QList<Summary> list;
        for (const QJsonValue &value : event.value(QStringLiteral("items")).toArray()) {
            const Summary summary = readSummary(value.toObject());
            if (!summary.id.isEmpty()) list << summary;
        }
        m_listed = true;
        const bool currentGone = !m_current.summary.id.isEmpty() && !m_expectNew &&
                                 std::none_of(list.cbegin(), list.cend(), [this](const Summary &summary) {
                                     return summary.id == m_current.summary.id;
                                 });
        m_list = list;
        emit listChanged();
        // Deleted while open: a new conversation in its place.
        if (currentGone && !m_current.items.isEmpty()) {
            m_chosen.clear();
            m_current = Conversation{};
            emit currentChanged();
        }
        return true;
    }
    if (type != QLatin1String("conversation")) return false;
    const QString id = event.value(QStringLiteral("id")).toString();
    if (id.isEmpty()) return true;
    if (m_expectNew) {
        // The conversation the request just sent started.
        m_expectNew = false;
        m_chosen = id;
    }
    if (id != m_chosen) return true;
    Conversation conversation;
    conversation.summary = readSummary(event);
    for (const QJsonValue &value : event.value(QStringLiteral("items")).toArray()) conversation.items << readItem(value.toObject());
    m_current = conversation;
    emit currentChanged();
    return true;
}

} // namespace kerf
