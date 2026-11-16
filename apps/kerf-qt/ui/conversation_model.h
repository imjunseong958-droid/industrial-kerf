#pragma once

// The conversations kept beside the project (.kerf/conversations), as the
// bridge reports them: the list the sidebar shows, and the one open in the
// conversation panel, whole. A pure model with no widgets.
//
// Inputs, all bridge data:
//   conversations {items: [{id, title, created, updated, archived, count}]}
//   conversation  {id, title, created, updated, archived, items: [...]}
// applyEvent(type, event) routes both.
//
// The open conversation is the one the person chose, or, after a request sent
// with none open (expectNew()), the first conversation the bridge reports
// next: the one that request started.

#include <QDateTime>
#include <QJsonObject>
#include <QList>
#include <QObject>
#include <QString>
#include <QStringList>

#include <limits>

namespace kerf {

class ConversationModel final : public QObject {
    Q_OBJECT
public:
    // One line of a conversation: the person's request (you) or Kerf's answer.
    struct Item {
        QString id;
        bool you = false;
        QString text;
        QDateTime at;
        double timecode = std::numeric_limits<double>::quiet_NaN(); // where the playhead was; NaN when none
        QStringList steps;  // what Kerf did on the way, one line each
        double seconds = 0; // how long Kerf worked
        bool error = false;
        bool stopped = false;
        QString proposal; // the proposal the turn staged
        bool hasTimecode() const { return timecode == timecode; }
    };
    struct Summary {
        QString id;
        QString title;
        QDateTime created;
        QDateTime updated;
        bool archived = false;
        int count = 0;
    };
    struct Conversation {
        Summary summary;
        QList<Item> items;
        bool isEmpty() const { return summary.id.isEmpty(); }
    };

    explicit ConversationModel(QObject *parent = nullptr);

    // conversations or conversation; false for any other event.
    bool applyEvent(const QString &type, const QJsonObject &event);

    // Newest first, as the bridge lists them.
    const QList<Summary> &list() const { return m_list; }
    const Summary *summary(const QString &id) const;

    // The open conversation; empty for a new one.
    const Conversation &current() const { return m_current; }
    QString currentId() const { return m_current.summary.id; }
    // Opens `id` as soon as the bridge sends it (the wiring asks for it); an
    // empty id starts a new conversation.
    void choose(const QString &id);
    // The next conversation reported is the one a request just started.
    void expectNew() { m_expectNew = true; }
    bool expectsNew() const { return m_expectNew; }
    // Whether the list has been heard yet.
    bool listed() const { return m_listed; }

    static Item readItem(const QJsonObject &item);
    static QDateTime readTime(const QString &text);

signals:
    void listChanged();
    // The open conversation changed: another one, or more of it.
    void currentChanged();

private:
    QList<Summary> m_list;
    Conversation m_current;
    QString m_chosen;
    bool m_expectNew = false;
    bool m_listed = false;
};

} // namespace kerf
