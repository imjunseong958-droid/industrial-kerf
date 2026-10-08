#pragma once

#include <QHash>
#include <QJsonArray>
#include <QJsonObject>
#include <QWidget>

class QDoubleSpinBox;
class QFormLayout;
class QLabel;

namespace cut {

// Effect Controls: the selected clip's timing and the catalog properties of
// its track type or generator. Edits leave as typed operations counted in
// frames of the sequence; values commit on focus loss.
class EffectControlsPanel final : public QWidget {
    Q_OBJECT
    friend class EditorTest;
public:
    explicit EffectControlsPanel(QWidget *parent = nullptr);

    // Show `clip` with editors for `defs`; an empty id shows nothing to edit.
    void setClip(const QString &id, const QJsonObject &clip, const QJsonArray &defs, double fps);

signals:
    void editRequested(const QJsonArray &ops);

private:
    QDoubleSpinBox *timingSpin(double minimum, double maximum, double step, const QString &field);
    QWidget *propertyEditor(const QJsonObject &def);
    static void setPropertyValue(QWidget *editor, const QJsonValue &value);
    void timingEdited();
    // A catalog property edit; the default value clears the property.
    void propertyEdited(const QString &name, const QJsonValue &value);
    void edit(const QJsonObject &op);

    QString m_clipId;
    QJsonObject m_clip;
    double m_fps = 24.0;
    bool m_updating = false;
    QLabel *m_clipName = nullptr;
    QDoubleSpinBox *m_position = nullptr;
    QDoubleSpinBox *m_duration = nullptr;
    QDoubleSpinBox *m_in = nullptr;
    QDoubleSpinBox *m_out = nullptr;
    QDoubleSpinBox *m_speed = nullptr;
    QFormLayout *m_propertyForm = nullptr;
    QJsonArray m_propertyDefs;
    QHash<QString, QWidget *> m_propertyEditors;
};

} // namespace cut
