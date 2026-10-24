#pragma once

#include <QHash>
#include <QJsonArray>
#include <QJsonObject>
#include <QWidget>

#include <functional>

class QAction;
class QDoubleSpinBox;
class QFormLayout;
class QHBoxLayout;
class QLabel;
class QVBoxLayout;

namespace kerf {

// Effect Controls: the selected clip's timing, the catalog properties of
// its track type or generator, a picture clip's masks with the tools that
// draw them on the Program monitor, and its effect stack, each effect with
// its switch, removal, and parameters. Edits leave as typed operations
// counted in frames of the sequence; values commit on focus loss.
class EffectControlsPanel final : public QWidget {
    Q_OBJECT
    friend class EditorTest;
public:
    explicit EffectControlsPanel(QWidget *parent = nullptr);

    // Show `clip` with editors for `defs`; an empty id shows nothing to edit.
    void setClip(const QString &id, const QJsonObject &clip, const QJsonArray &defs, double fps);
    // The catalog from kerf_fx_catalog, for the effects' parameters.
    void setEffectCatalog(const QJsonArray &catalog);
    // The picture tracks an effect on the clip may read (track parameters)
    // and the sound tracks it may draw (sound parameters).
    void setPictureTracks(const QStringList &tracks);
    void setSoundTracks(const QStringList &tracks);
    // The mask tools, as buttons beside the clip's masks.
    void setMaskTools(const QList<QAction *> &tools);
    // A mask's property definitions and modes, from the snapshot.
    void setMaskCatalog(const QJsonArray &defs, const QStringList &modes);
    // Whether the clip shown next carries masks: a picture clip does.
    void setMaskable(bool maskable);
    void selectMask(const QString &name);

signals:
    void editRequested(const QJsonArray &ops);
    // A mask's name was clicked.
    void maskSelected(const QString &name);

private:
    QDoubleSpinBox *timingSpin(double minimum, double maximum, double step, const QString &field);
    QWidget *propertyEditor(const QJsonObject &def);
    QWidget *valueEditor(const QJsonObject &def, const QString &title, std::function<void(const QJsonValue &)> edited);
    void showEffects();
    void showMasks();
    // A change to mask `name`: its mode, inversion, or properties.
    void maskEdited(const QString &name, const QJsonObject &change);
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
    QVBoxLayout *m_effectsLayout = nullptr;
    QHash<QString, QJsonObject> m_effectDefs;
    QStringList m_pictureTracks;
    QStringList m_soundTracks;
    // The effects shown, as clip and stack, to rebuild only when they change.
    QString m_effectsShown;
    QWidget *m_masksSection = nullptr;
    QHBoxLayout *m_maskHeader = nullptr;
    QVBoxLayout *m_masksLayout = nullptr;
    QLabel *m_masksHint = nullptr;
    QJsonArray m_maskDefs;
    QStringList m_maskModes;
    // The clip's masks by name, which mask parameters pick from.
    QStringList m_maskNames;
    bool m_maskable = false;
    QString m_selectedMask;
    // The masks shown, as clip and names, to rebuild only when they change.
    QString m_masksShown;
};

} // namespace kerf
