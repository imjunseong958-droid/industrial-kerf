#pragma once

// What Settings' pages are made of. A page is its name and, under it, rows:
// each a setting's name with a line saying what it does, and its control at
// the row's end. Rows that belong together stand one under another with a
// hairline between them, and nothing is boxed.
//
//   Appearance                               the page's name, UI 18 medium
//
//   Interface font           [Inter|Pretendard|System]    a row: 12 above and
//   Pretendard adds Hangul…                               below, name 14 ink,
//   ─────────────────────────────────────────────────     what it does 13 ink3
//   Conversation font              [Same as interface|Serif]
//
// A row may carry something wide under its text (a field of JSON, a list),
// and a group may be named by a quiet label over its first row.

#include "components.h"

#include <QScrollArea>
#include <QWidget>

class QLabel;
class QLayout;
class QVBoxLayout;
class QVariantAnimation;

namespace kerf::ui {

// On or off, in force at once: a 32 x 18 track, the accent while on, and a
// 14 pt knob that slides to the other end (MOTION_UI.md, Switch).
class Switch final : public KitButton {
    Q_OBJECT

public:
    explicit Switch(QWidget *parent = nullptr);
    QSize sizeHint() const override;
    QSize minimumSizeHint() const override { return sizeHint(); }
    // Where the knob is: 0 off, 1 on, between while it slides.
    qreal position() const { return m_position; }

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    QVariantAnimation *m_slide = nullptr;
    qreal m_position = 0;
};

class SettingsRow final : public QWidget {
    Q_OBJECT

public:
    explicit SettingsRow(const QString &label, const QString &description = {}, QWidget *control = nullptr,
                         QWidget *parent = nullptr);
    QString label() const;
    // The line under the name; empty, it takes no room.
    QString description() const;
    void setDescription(const QString &description);
    QLabel *descriptionLabel() const { return m_description; }
    QWidget *control() const { return m_control; }
    // Something wide, under the text and across the row.
    void addBelow(QWidget *widget);
    void addBelow(QLayout *layout);
    // A hairline over the row: it follows another in its group.
    void setLineAbove(bool line);
    bool hasLineAbove() const { return m_lineAbove; }

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    QVBoxLayout *m_column = nullptr;
    QLabel *m_label = nullptr;
    QLabel *m_description = nullptr;
    QWidget *m_control = nullptr;
    bool m_lineAbove = false;
};

class SettingsGroup final : public QWidget {
    Q_OBJECT

public:
    explicit SettingsGroup(const QString &label = {}, QWidget *parent = nullptr);
    SettingsRow *addRow(const QString &label, const QString &description = {}, QWidget *control = nullptr);
    // Rows that come and go together, after this group's own: shown or
    // hidden as one, the first of them under a hairline like any other.
    SettingsGroup *addRows();
    QList<SettingsRow *> rows() const { return m_rows; }

private:
    QVBoxLayout *m_column = nullptr;
    QList<SettingsRow *> m_rows;
    // Whether its first row follows a row of the group it is in.
    bool m_continues = false;
};

class SettingsPage final : public QScrollArea {
    Q_OBJECT

public:
    explicit SettingsPage(const QString &title, QWidget *parent = nullptr);
    QString title() const;
    // Rows that belong together; `label` names them, quietly, over the first.
    SettingsGroup *addGroup(const QString &label = {});

private:
    QVBoxLayout *m_column = nullptr;
    QLabel *m_title = nullptr;
};

} // namespace kerf::ui
