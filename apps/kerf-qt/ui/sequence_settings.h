#pragma once

// Sequence Settings… (sequence.settings, ⌘0; mockup-spec 11, the Sequence
// menu): a flat dialog over the program sequence's own settings, sent to
// kerfd as one `set-sequence` edit with only what changed.
//
//   Name          {name}
//   Format        16:9, 9:16, 1:1, 4:5 or a custom W × H   {width, height}
//   Start         the start timecode, `;` before the frames for drop-frame
//                 {start: "01:00:00:00"}
//   Frame rate    {rate: {num, den}} (rows matched to the snapshot's rate
//                 by value: kerfd writes "29.97", reads "30000/1001"), only while the sequence is empty (no
//                 clips, no markers); otherwise read-only with the note
//                 "Frame rate can change only while the sequence is empty",
//                 which is also kerfd's refusal.
//
// The dialog collects; ToolbarWiring sends the edit and shows kerfd's answer
// here (setError), closing it once the snapshot has the change.

#include "test_friends.h"

#include <QDialog>
#include <QJsonObject>
#include <QList>
#include <QPair>
#include <QSize>
#include <QString>

#include <optional>

class QComboBox;
class QLabel;
class QLineEdit;
class QSpinBox;

namespace kerf {

namespace ui {
class Button;
class SegmentedControl;
} // namespace ui

class SequenceSettingsDialog final : public QDialog {
    Q_OBJECT
    KERF_TEST_FRIENDS

public:
    static constexpr int kWidth = 400;

    // The program sequence as the snapshot's timeline describes it.
    struct Sequence {
        QString id;
        QString name;
        QSize size;
        QString rate; // as the snapshot writes it (Rate's Display): "25", "29.97"
        double fps = 0;
        double start = 0; // seconds
        bool dropFrame = false;
        bool empty = true; // no clips and no markers
    };
    static Sequence fromTimeline(const QJsonObject &timeline);
    // kerfd's refusal of a rate change for a sequence with content.
    static QString rateNote();
    // The rates offered: their label and the rate as kerfd writes it.
    static QList<QPair<QString, QString>> rates();
    // A rate in either spelling kerfd reads, as Rate::parse reads it:
    // "29.97" and "30000/1001" are both 30000/1001; "25" is 25/1.
    static std::optional<QPair<qint64, qint64>> rateFraction(const QString &rate);
    // Whether two spellings are the same rate ("29.97" and "30000/1001").
    static bool sameRate(const QString &a, const QString &b);
    // "29.97" or "30000/1001" → {num: 30000, den: 1001}; "25" → {num: 25,
    // den: 1}; empty for text that is no rate.
    static QJsonObject rateValue(const QString &rate);
    // The start as the field shows it: frames at the rate, drop-frame or not.
    static QString startText(const Sequence &sequence);
    // Whether the timeline's program sequence already says what `op` (a
    // set-sequence) asks for: how a newer snapshot shows kerfd took it.
    static bool holds(const QJsonObject &timeline, const QJsonObject &op);

    explicit SequenceSettingsDialog(const QJsonObject &timeline, QWidget *parent = nullptr);

    // A newer snapshot while the dialog is open: the frame rate follows
    // whether the sequence is still empty; edited fields keep what was typed.
    void setTimeline(const QJsonObject &timeline);
    Sequence sequence() const { return m_sequence; }

    // The set-sequence op for what changed ({"op": "set-sequence", ...}),
    // empty when nothing did; `problem` says what cannot be sent.
    QJsonObject op(QString *problem = nullptr) const;
    bool rateEditable() const;
    // kerfd's answer to an Apply that it refused; empty clears it.
    void setError(const QString &message);
    QString error() const;
    // Apply: the op goes out (applyRequested) unless nothing changed, which
    // just closes the dialog.
    void apply();

    // ---- the fields (tests and the window set them as a person would)
    QLineEdit *nameField() const { return m_name; }
    ui::SegmentedControl *formatControl() const { return m_format; }
    QSpinBox *widthField() const { return m_widthField; }
    QSpinBox *heightField() const { return m_heightField; }
    QLineEdit *startField() const { return m_start; }
    QComboBox *rateField() const { return m_rate; }
    QLabel *rateNoteLabel() const { return m_rateNote; }
    ui::Button *applyButton() const { return m_apply; }
    // The size chosen: a preset's, or the custom W × H.
    QSize chosenSize() const;

signals:
    void applyRequested(const QJsonObject &op);

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    void chooseFormat(int index);
    void syncRate();
    void syncApply();
    // The row holding a rate, matched by value; -1 when none does.
    int findRate(const QString &rate) const;
    // Select the sequence's rate, adding a row for it if none matches.
    void showRate(const Sequence &sequence);

    Sequence m_sequence;
    QLineEdit *m_name = nullptr;
    ui::SegmentedControl *m_format = nullptr;
    QWidget *m_custom = nullptr;
    QSpinBox *m_widthField = nullptr;
    QSpinBox *m_heightField = nullptr;
    QLineEdit *m_start = nullptr;
    QComboBox *m_rate = nullptr;
    QLabel *m_rateNote = nullptr;
    QLabel *m_error = nullptr;
    ui::Button *m_apply = nullptr;
    ui::Button *m_cancel = nullptr;
};

} // namespace kerf
